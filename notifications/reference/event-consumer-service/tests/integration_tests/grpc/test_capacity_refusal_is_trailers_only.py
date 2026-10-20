# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Wire-level proof that a capacity refusal is a trailers-only response.

WHAT THIS PROVES. When ``ConsumeNonDurableEvents`` turns a client away for
capacity, this pod puts exactly one HEADERS frame on that RPC's HTTP/2
stream, that frame carries END_STREAM, and no DATA frame precedes it --
i.e. the refusal is a textbook trailers-only response, emitted before any
part of a response has been committed. Both non-durable refusal classes
are covered: the memory-safe stream cap (which refuses every class) and
the re-routable shed threshold (which refuses proxy-stamped streams
inside the overflow band, SDD SR-3.7). The shed is the one Envoy's
``retryOn: resource-exhausted`` exists for, so it is the class that most
needs the property; the cap is covered too because the two share the
whole refusal path.

WHY IT MATTERS. Envoy decides whether a gRPC response is retryable from
the response headers alone. Once response headers commit, the retry is
off the table -- so the guard call at the top of ``ConsumeNonDurableEvents``
(``event_consumer_servicer.py``, first statement, ahead of initial
metadata and the first yield) is what makes the shed re-routable at all.
That ordering is invisible to a status-code assertion: moving the guard
below anything that commits a response would keep every
RESOURCE_EXHAUSTED test in this directory green while silently breaking
re-routing. This file is the test that fails instead.

Task 4's Breakdown contract recorded the trailers-only behaviour of
grpc.aio's abort-before-any-yield as an INFERENCE. It is measured here
rather than inferred, and it needs no deployed proxy: the harness and
Contour's ``h2c`` route to the pod both speak cleartext HTTP/2, so the
frames a TCP interposer sees between client and server are the frames
Envoy sees.

WHAT THIS DOES NOT PROVE. That the proxy then re-routes the shed stream.
Envoy's retry is a fresh load-balancer selection that can land elsewhere
-- Contour sets no retry-host predicate, so landing on a *different* pod
is likely-but-not-guaranteed, and nothing here observes it. The rendered
``retryPolicy`` that asks for the retry is pinned by Task 4's helm-template
test (``tests/helm/test_http_proxy_streaming_template.py``), and the
end-to-end behaviour against a real proxy is this Task's separate
deployed-proxy evidence Subtask. Two further exclusions come straight
from Task 4's contract and are equally out of scope here: a refusal
raised after the servicer has started responding is NOT retryable (no
such refusal exists on this path today, which is exactly what the
first-statement guard call buys), and a client that pushed more than
Envoy's ~1 MiB retry buffer before admission is past the point where a
retry can replay its request.

HOW THE MEASUREMENT WORKS. An ``asyncio`` TCP interposer sits between the
client and the ``grpc_server`` fixture, forwarding bytes both ways and
recording the 9-byte HTTP/2 frame header of every frame in the SERVER ->
CLIENT direction. No HPACK decoder and no new dependency are involved
(``httpx`` is in the lockfile without the ``h2`` extra; ``h2`` / ``hpack``
/ ``hyperframe`` are absent): frame *headers* are fixed-layout plaintext
-- 3-byte length, 1-byte type, 1-byte flags, 4-byte stream id -- and the
only fields this file reads are type, flags, and stream id. The header
BLOCK that follows is HPACK-compressed and is never inspected, so the
counting argument stands on frame boundaries alone.

The whole approach would be worthless if the recorder reported one
HEADERS frame no matter what, so ``TestTheRecorderDiscriminatesAnAdmitted
Stream`` drives an ADMITTED stream through the same interposer and pins
the opposite shape: more than one HEADERS frame (initial metadata, then
trailers) with DATA frames between them.
"""

import asyncio
import contextlib
import socket
from dataclasses import dataclass, field

import grpc
import pytest
import pytest_asyncio
from grpc.aio import AioRpcError, insecure_channel
from notification_common.rabbitmq.rabbitmq_connector import RabbitMQConnector

from app.config import settings
from app.lib.rabbitmq import capacity_guard
from app.lib.rabbitmq.capacity_guard import CapacityGuard
from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2_grpc
from nvidia.omniverse.notifications.consumer.v1beta.event_consumer_pb2 import (
    ConsumeNonDurableEventsRequest,
)
from tests.app.conftest import _make_pool_stats_by_pool
from tests.integration_tests.conftest import filter_group

RESOURCE_EXHAUSTED = grpc.StatusCode.RESOURCE_EXHAUSTED

# HTTP/2 frame vocabulary (RFC 9113 section 6). Only the three values this
# file discriminates on are named.
HEADERS_FRAME = 0x1
DATA_FRAME = 0x0
END_STREAM_FLAG = 0x1

# Every frame is prefixed by a fixed 9-byte header: 3-byte payload length,
# 1-byte type, 1-byte flags, 4-byte stream id (high bit reserved).
FRAME_HEADER_BYTES = 9
STREAM_ID_MASK = 0x7FFFFFFF

# Stream 0 carries connection-level frames only -- SETTINGS, PING,
# WINDOW_UPDATE, GOAWAY -- none of which belong to any RPC.
CONNECTION_STREAM_ID = 0

READ_CHUNK_BYTES = 65536

# The stamp _is_proxied reads to classify a stream as re-routable. Sent as
# real request metadata so the class is derived the way production derives
# it, rather than by patching the classifier.
PROXY_HEADER = "x-via-proxy"
PROXY_HEADER_VALUE = "true"

# Pool geometry. _make_pool_stats_by_pool's defaults build a non-durable
# pool of max_pool_size 100 x channel_client_limit 1, so the pool's own
# stream capacity -- the SOFT threshold -- is 100.
POOL_STREAM_CAPACITY = 100

# The cap-refusal row: a hard bound far below the pool's stream capacity,
# with the pod sitting exactly on it, so the refusal is unambiguously the
# cap's (streams >= hard_bound) and not the shed's.
STREAM_CAP = 5

# The shed-refusal row, derived from that capacity so the geometry stays
# readable: a hard bound above the pool's stream capacity opens a real
# overflow band, the shed threshold is placed explicitly inside it rather
# than left to the midpoint default (so these numbers do not move if that
# default is retuned), and the pod sits above the shed threshold but
# below the hard bound. A proxy-stamped client is shed there; an
# unstamped one would still be admitted, which is what makes this row the
# shed's and not the cap's -- that admitted/shed pairing is AC-12's own
# coverage (``test_ac12_class_shed.py``), not restated here.
SHED_HARD_BOUND = 2 * POOL_STREAM_CAPACITY
SHED_THRESHOLD = POOL_STREAM_CAPACITY + 20
STREAMS_INSIDE_THE_SHED_BAND = SHED_THRESHOLD + 10

# A cap far above anything the pool can hold, for the admitted row.
NON_BINDING_CAP = 1_000_000

# The reconnect-token response followed by the three events the autouse
# mock_queue_iterator streams.
EXPECTED_ADMITTED_RESPONSE_COUNT = 4


@dataclass(frozen=True)
class ServerFrame:
    """
    One HTTP/2 frame header seen travelling server -> client.

    :param frame_type: The frame's type byte (``HEADERS_FRAME``,
        ``DATA_FRAME``, or any other RFC 9113 type).
    :param flags: The frame's flags byte; ``END_STREAM_FLAG`` is the bit
        this file reads.
    :param stream_id: The RPC stream the frame belongs to, or
        ``CONNECTION_STREAM_ID`` for connection-level frames.
    """

    frame_type: int
    flags: int
    stream_id: int


@dataclass
class ServerFrameRecorder:
    """
    Reassemble the server -> client byte stream into frame headers.

    HTTP/2 frames are not aligned to TCP reads: one read can carry half a
    frame, or three frames and a fragment. So bytes are appended to a
    running buffer and consumed 9 + length at a time; a frame is recorded
    only once its whole payload has arrived. Inspecting the first nine
    bytes of each chunk instead would miscount every frame after the
    first in a coalesced read.

    Only frame HEADERS are parsed. The header BLOCK inside a HEADERS
    frame is HPACK-compressed, and nothing here needs its contents -- the
    property under test is a statement about frame counts, types, and
    flags, so no HPACK decoder (and no new dependency) is required.
    """

    frames: list = field(default_factory=list)
    _pending: bytearray = field(default_factory=bytearray)

    def feed(self, chunk):
        """
        Record every whole frame in ``chunk``, buffering any remainder.

        :param chunk: Bytes just read from the server, in order.
        """
        self._pending.extend(chunk)

        while len(self._pending) >= FRAME_HEADER_BYTES:
            payload_length = int.from_bytes(self._pending[0:3], "big")
            frame_length = FRAME_HEADER_BYTES + payload_length

            if len(self._pending) < frame_length:
                return

            self.frames.append(
                ServerFrame(
                    frame_type=self._pending[3],
                    flags=self._pending[4],
                    stream_id=int.from_bytes(self._pending[5:9], "big")
                    & STREAM_ID_MASK,
                )
            )
            del self._pending[:frame_length]

    def frames_by_stream(self):
        """
        Group the recorded RPC frames by stream id.

        Connection-level frames (stream 0) are dropped: SETTINGS and
        WINDOW_UPDATE traffic belongs to the connection, not to any RPC,
        and counting it would swamp the per-RPC totals.

        :return: ``{stream_id: [ServerFrame, ...]}`` in arrival order.
        """
        by_stream = {}

        for frame in self.frames:
            if frame.stream_id == CONNECTION_STREAM_ID:
                continue

            by_stream.setdefault(frame.stream_id, []).append(frame)

        return by_stream

    def first_rpc_frames(self):
        """
        Return the frames of the first RPC opened on this connection.

        Grouping by stream id and selecting one stream is what keeps the
        counts honest: a flat "stream id != 0" filter would merge two
        RPCs sharing the connection into one total, and an admitted
        stream's frames would then hide a refused stream's shape. Client
        stream ids are assigned in ascending order, so the lowest is the
        first RPC the test opened.

        :return: That stream's frames in arrival order, or ``[]`` if the
            server sent nothing on any RPC stream.
        """
        by_stream = self.frames_by_stream()

        if not by_stream:
            return []

        return by_stream[min(by_stream)]


def headers_frames(frames):
    """Return only the HEADERS frames of ``frames``."""
    return [frame for frame in frames if frame.frame_type == HEADERS_FRAME]


def data_frames(frames):
    """Return only the DATA frames of ``frames``."""
    return [frame for frame in frames if frame.frame_type == DATA_FRAME]


@dataclass
class InterposedChannel:
    """
    A stub whose traffic is recorded at the wire.

    :param stub: An ``EventConsumerServiceStub`` dialled at the
        interposer, not at the server -- calls made on the shared
        ``event_consumer_stub`` fixture bypass the recorder entirely.
    :param recorder: The :class:`ServerFrameRecorder` fed every byte the
        server sends back, BEFORE those bytes are forwarded to the
        client. That ordering is the synchronization: by the time a test
        observes an RPC's status or its last response, the frames that
        carried it are already recorded, so nothing has to wait or sleep.
    """

    stub: object
    recorder: ServerFrameRecorder


async def _pump(reader, writer, on_chunk=None):
    """
    Forward one direction of a connection until it closes.

    :param reader: The side being read from.
    :param writer: The side being written to.
    :param on_chunk: Optional callback given each chunk before it is
        forwarded.
    """
    try:
        while True:
            chunk = await reader.read(READ_CHUNK_BYTES)

            if not chunk:
                break

            if on_chunk is not None:
                on_chunk(chunk)

            writer.write(chunk)
            await writer.drain()
    except ConnectionResetError:
        # The client channel closing under an in-flight pump is the
        # normal end of a test, not a failure.
        pass
    finally:
        with contextlib.suppress(ConnectionResetError, OSError):
            writer.close()


@pytest_asyncio.fixture
async def interposed_stub(grpc_server, grpc_host):
    """
    Yield a stub that reaches the real server through a frame recorder.

    The interposer listens on port 0 -- the OS picks a free port, so
    nothing collides under ``pytest-xdist`` -- and forwards to the
    ``grpc_server`` fixture's own port.

    It binds the interface the CLIENT will dial (``grpc_host``, which is
    ``host.docker.internal`` under ``app_env == "test"`` and ``localhost``
    otherwise) and forwards to localhost, where the server bound. The
    listener is pinned to a single resolved address and the stub dials
    that address literally, because a hostname such as ``localhost``
    resolves to both ``::1`` and ``127.0.0.1``: binding by name with port
    0 would open two listeners on two DIFFERENT ports, and the client
    could pick the one the test never read a port from.
    """
    server, server_port = grpc_server
    recorder = ServerFrameRecorder()
    pumps = set()

    async def _handle(client_reader, client_writer):
        upstream_reader, upstream_writer = await asyncio.open_connection(
            "localhost", server_port
        )
        directions = (
            asyncio.create_task(_pump(client_reader, upstream_writer)),
            asyncio.create_task(_pump(upstream_reader, client_writer, recorder.feed)),
        )
        pumps.update(directions)
        await asyncio.gather(*directions, return_exceptions=True)

    family, _socktype, _proto, _canonname, sockaddr = socket.getaddrinfo(
        grpc_host, 0, type=socket.SOCK_STREAM
    )[0]
    listen_ip = sockaddr[0]
    interposer = await asyncio.start_server(
        _handle, host=listen_ip, port=0, family=family
    )
    listen_port = interposer.sockets[0].getsockname()[1]
    target = (
        f"[{listen_ip}]:{listen_port}"
        if family == socket.AF_INET6
        else f"{listen_ip}:{listen_port}"
    )

    async with insecure_channel(target) as channel:
        yield InterposedChannel(
            stub=event_consumer_pb2_grpc.EventConsumerServiceStub(channel),
            recorder=recorder,
        )

    interposer.close()

    for pump in pumps:
        pump.cancel()

    await asyncio.gather(*pumps, return_exceptions=True)
    await interposer.wait_closed()


@pytest.fixture
def real_guard(mocker):
    """
    Run the REAL guard over crafted pool stats for one test.

    Same binding pattern as ``test_memory_safe_stream_cap.py``: the
    autouse ``before_each`` patch of ``capacity_guard.raise_if_at_capacity``
    stays in place (the connector's pools are nulled out suite-wide, so a
    real guard call with no pool would fail closed), and its
    ``side_effect`` is pointed at a real ``CapacityGuard`` bound to a
    mock connector. Every argument the servicer passes -- including the
    ``re_routable`` class it derived from the request's own metadata --
    therefore reaches production admission arithmetic.

    That matters more here than anywhere else in the suite: a test that
    aborted from a hand-written servicer would be measuring grpc-core's
    framing of somebody else's abort, not the framing of OUR refusal.

    Both thresholds are stated explicitly by every caller. They are read
    per admission decision, so patching the settings takes effect on the
    next request.
    """

    def _install(stream_cap, non_durable_clients, shed_threshold=None):
        # spec'd against the real connector so a rename of
        # pool_stats_by_pool fails loudly here instead of silently
        # handing the guard a MagicMock it reads as "no pools".
        connector = mocker.MagicMock(spec=RabbitMQConnector)
        # Not draining: the guard reads ``connector.draining`` on the
        # non-durable path (Epic 5, Task 2 / SR-6.1). It is a plain instance
        # attribute absent from the class spec, so it must be set explicitly or
        # the guard's read raises ``AttributeError``. This scenario drives the
        # cap/shed refusal contract, not the draining short-circuit.
        connector.draining = False
        connector.pool_stats_by_pool.side_effect = lambda: _make_pool_stats_by_pool(
            non_durable_clients=non_durable_clients
        )

        mocker.patch.object(
            settings.settings, "max_non_durable_streams_per_pod", stream_cap
        )
        mocker.patch.object(
            settings.settings, "max_re_routable_streams_per_pod", shed_threshold
        )
        capacity_guard.raise_if_at_capacity.side_effect = CapacityGuard(
            connector=connector
        ).raise_if_at_capacity

    return _install


@dataclass(frozen=True)
class RefusalClass:
    """
    One of the two ways a non-durable stream is refused.

    :param stream_cap: The memory-safe stream cap to run under -- the
        hard bound, which refuses every class.
    :param shed_threshold: The re-routable shed threshold, or None to
        leave it at its band-midpoint default.
    :param non_durable_clients: Streams the pod is already carrying.
    :param proxy_stamped: Whether the client sends the X-Via-Proxy stamp,
        which is the single bit ``_is_proxied`` classifies the stream on.
    """

    stream_cap: int
    shed_threshold: int | None
    non_durable_clients: int
    proxy_stamped: bool


REFUSAL_CLASSES = {
    "cap": RefusalClass(
        stream_cap=STREAM_CAP,
        shed_threshold=None,
        non_durable_clients=STREAM_CAP,
        proxy_stamped=False,
    ),
    "shed": RefusalClass(
        stream_cap=SHED_HARD_BOUND,
        shed_threshold=SHED_THRESHOLD,
        non_durable_clients=STREAMS_INSIDE_THE_SHED_BAND,
        proxy_stamped=True,
    ),
}


@dataclass
class RefusedCall:
    """
    Everything one refused RPC left behind.

    :param code: The status code the client saw.
    :param initial_metadata: The initial metadata the client saw, as a
        list of pairs -- empty when the server committed no response
        headers of its own.
    :param messages: Response messages the client received before the
        status; any at all would mean the response had already committed.
    :param frames: The server -> client frames recorded on this RPC's
        HTTP/2 stream.
    """

    code: grpc.StatusCode
    initial_metadata: list
    messages: list
    frames: list


def _no_requests():
    """
    Build the client-side request stream for a refused RPC: empty.

    The endpoint checks capacity before it reads a request, so a client
    write landing after the server aborted fails on the CLIENT side with
    a status of grpc-core's own making, masking the RESOURCE_EXHAUSTED
    the server really sent (``test_capacity_guard.py`` has the full
    account of that race). Sending nothing cannot make a refusal test
    pass for the wrong reason: an ADMITTED RPC with an empty request
    stream just returns, and the absent exception fails the test.
    """
    return iter(())


def _one_filter_request():
    """Build the client-side request stream for an admitted RPC."""
    request = ConsumeNonDurableEventsRequest()
    request.filter_groups.append(filter_group(event_type="e1"))
    yield request


@pytest.fixture(params=sorted(REFUSAL_CLASSES), ids=sorted(REFUSAL_CLASSES))
def refusal_class(request):
    """Parametrize over both non-durable refusal classes."""
    return REFUSAL_CLASSES[request.param]


@pytest_asyncio.fixture
async def refused_call(interposed_stub, refusal_class, real_guard, metadata):
    """
    Drive one refused ``ConsumeNonDurableEvents`` and collect the evidence.

    The refusal is real admission arithmetic reached through real request
    metadata: the shed row is refused because the client stamped itself
    proxied and the pod sits inside the overflow band, the cap row
    because the pod sits on the hard bound.
    """
    real_guard(
        refusal_class.stream_cap,
        non_durable_clients=refusal_class.non_durable_clients,
        shed_threshold=refusal_class.shed_threshold,
    )

    call_metadata = list(metadata)

    if refusal_class.proxy_stamped:
        call_metadata.append((PROXY_HEADER, PROXY_HEADER_VALUE))

    call = interposed_stub.stub.ConsumeNonDurableEvents(
        _no_requests(), metadata=call_metadata
    )
    messages = []

    with pytest.raises(AioRpcError) as exc_info:
        async for response in call:
            messages.append(response)

    return RefusedCall(
        code=exc_info.value.code(),
        initial_metadata=list(await call.initial_metadata()),
        messages=messages,
        frames=interposed_stub.recorder.first_rpc_frames(),
    )


@pytest_asyncio.fixture
async def admitted_call_frames(interposed_stub, real_guard, metadata):
    """
    Drive one ADMITTED stream through the same interposer.

    The pod is idle under a non-binding cap, so admission cannot refuse;
    the RPC is drained to completion, and the response count is asserted
    here so a stream that was quietly refused (or never driven) cannot
    reach the tests as an "admitted" one.
    """
    real_guard(NON_BINDING_CAP, non_durable_clients=0)

    responses = [
        response
        async for response in interposed_stub.stub.ConsumeNonDurableEvents(
            _one_filter_request(), metadata=metadata
        )
    ]

    assert len(responses) == EXPECTED_ADMITTED_RESPONSE_COUNT

    return interposed_stub.recorder.first_rpc_frames()


class TestCapacityRefusalIsTrailersOnly:
    """
    The wire property, on both non-durable refusal classes.

    A trailers-only response is exactly one HEADERS frame carrying
    END_STREAM with no DATA frame on the stream. Anything else -- a
    second HEADERS frame, or a DATA frame before the trailers -- means
    response headers committed first, which is precisely the state in
    which Envoy will no longer retry.
    """

    @pytest.mark.asyncio
    async def test_refusal_emits_exactly_one_headers_frame(self, refused_call):
        assert len(headers_frames(refused_call.frames)) == 1

    @pytest.mark.asyncio
    async def test_refusal_headers_frame_carries_end_stream(self, refused_call):
        # One HEADERS frame that did NOT end the stream would be initial
        # metadata with the trailers still to come -- a committed
        # response, not a trailers-only one.
        headers = headers_frames(refused_call.frames)

        assert headers[0].flags & END_STREAM_FLAG

    @pytest.mark.asyncio
    async def test_refusal_emits_no_data_frames(self, refused_call):
        assert data_frames(refused_call.frames) == []


class TestClientSeesNoCommittedResponse:
    """
    The client-observable corollary of the same refusal.

    Necessary rather than sufficient -- grpc-core's implicit empty
    HEADERS frame is invisible to the Python API, so a client cannot tell
    a trailers-only refusal from an abort taken after a message purely
    from its initial metadata. That is what the frame assertions above
    are for. These are asserted separately because they are the
    client-facing contract and the part that survives if the interposer
    ever has to be retired.
    """

    @pytest.mark.asyncio
    async def test_refusal_status_is_resource_exhausted(self, refused_call):
        assert refused_call.code == RESOURCE_EXHAUSTED

    @pytest.mark.asyncio
    async def test_refusal_delivers_empty_initial_metadata(self, refused_call):
        assert refused_call.initial_metadata == []

    @pytest.mark.asyncio
    async def test_refusal_delivers_no_messages(self, refused_call):
        assert refused_call.messages == []


class TestTheRecorderDiscriminatesAnAdmittedStream:
    """
    Non-vacuity: the recorder is not answering "one HEADERS frame" blind.

    A recorder that dropped every frame but the first, or that never
    reassembled across TCP reads, would satisfy every assertion above
    while measuring nothing. An admitted stream through the same
    interposer must therefore record the OPPOSITE shape: initial
    metadata, response DATA, then trailers.
    """

    @pytest.mark.asyncio
    async def test_admitted_stream_records_more_than_one_headers_frame(
        self, admitted_call_frames
    ):
        assert len(headers_frames(admitted_call_frames)) > 1

    @pytest.mark.asyncio
    async def test_admitted_stream_records_data_frames(self, admitted_call_frames):
        assert len(data_frames(admitted_call_frames)) >= 1
