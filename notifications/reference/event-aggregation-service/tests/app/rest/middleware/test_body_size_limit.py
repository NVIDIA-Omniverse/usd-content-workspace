# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Unit tests for the body-size-limit ASGI middleware.

These drive the middleware directly at the ASGI level -- a scope, a queued
`receive`, a recording `send`, and a recording inner application -- with no
server and no FastAPI stack in between. That is the only layer at which the
two properties that matter can be seen: that the inner application is never
invoked for a refused request, and that an accepted streamed body reaches it
intact after having been drained and replayed. The end-to-end wiring (the
middleware installed outermost on the real app, the cap read from settings) is
pinned by the REST integration tests instead.

The cap here is a small `MAX_BYTES` rather than the 4 MiB production default so
each case is a handful of bytes; nothing in the middleware is sensitive to the
magnitude of the bound.
"""

import json
import logging

import pytest
import pytest_asyncio

from app.rest.middleware.body_size_limit import BodySizeLimitMiddleware

MAX_BYTES = 10
UNDER_CAP_BYTES = MAX_BYTES - 1
UNDER_CAP = str(UNDER_CAP_BYTES).encode()
LOGGER_NAME = "app.rest.middleware.body_size_limit"
REQUEST_PATH = "/api/v1beta/events"

DISCONNECT = {"type": "http.disconnect"}

# Content-length declarations the middleware must not trust: a duplicated
# header (the classic request-smuggling shape), a negative value, and a
# non-numeric one. Each declares -- or, duplicated, agrees on -- a size well
# inside the cap, so a middleware that trusted the declaration would wave an
# oversize body straight through to the inner application. The counting branch
# refuses it instead.
UNTRUSTWORTHY_DECLARATIONS = [
    (b"1", b"1"),
    (b"-1",),
    (b"not-a-number",),
]
UNTRUSTWORTHY_IDS = ["duplicated", "negative", "non_numeric"]


class RecordingApp:
    """An inner ASGI app that records how it was invoked.

    On an ``http`` scope it drains the body through the `receive` it was handed,
    takes `trailing_reads` further reads past the end of the body, and answers
    200. On any other scope it records the invocation and returns.
    """

    def __init__(self, trailing_reads: int = 0) -> None:
        self.scopes: list = []
        self.receives: list = []
        self.received: list = []
        self.trailing: list = []
        self._trailing_reads = trailing_reads

    async def __call__(self, scope, receive, send) -> None:
        self.scopes.append(scope)
        self.receives.append(receive)
        if scope["type"] != "http":
            return
        await self._drain(receive)
        for _ in range(self._trailing_reads):
            self.trailing.append(await receive())
        await send({"type": "http.response.start", "status": 200, "headers": []})
        await send({"type": "http.response.body", "body": b"inner"})

    async def _drain(self, receive) -> None:
        while True:
            message = await receive()
            self.received.append(message)
            if message["type"] != "http.request":
                return
            if not message.get("more_body", False):
                return

    @property
    def was_called(self) -> bool:
        return bool(self.scopes)


class QueuedReceive:
    """An ASGI receive handing back queued messages, then a disconnect.

    The disconnect tail stands in for the real server's behavior once a body is
    exhausted, and lets a test see whether the middleware's replay really does
    fall through to the underlying receive.
    """

    def __init__(self, *messages) -> None:
        self._queue = list(messages)
        self.call_count = 0

    async def __call__(self):
        self.call_count += 1
        if self._queue:
            return self._queue.pop(0)
        return DISCONNECT


class RecordingSend:
    """An ASGI send collecting the response messages it is given."""

    def __init__(self) -> None:
        self.messages: list = []

    async def __call__(self, message) -> None:
        self.messages.append(message)

    @property
    def status(self) -> int:
        return self.messages[0]["status"]

    @property
    def headers(self) -> dict:
        return dict(self.messages[0]["headers"])

    @property
    def body(self) -> bytes:
        return self.messages[1]["body"]


def http_scope(*content_lengths: bytes) -> dict:
    """Return an http scope declaring a content-length header per argument."""
    headers = [(b"content-type", b"application/json")]
    headers.extend((b"content-length", value) for value in content_lengths)
    return {"type": "http", "path": REQUEST_PATH, "headers": headers}


def body_message(size: int, more_body: bool = False) -> dict:
    """Return an http.request message carrying `size` bytes of body."""
    return {"type": "http.request", "body": b"x" * size, "more_body": more_body}


def refusals(caplog) -> list:
    """Return the warning records the middleware itself logged.

    Setup records are included alongside the call-phase ones because the
    refusal classes below drive the middleware from an autouse fixture, and
    `caplog.records` holds only the current phase's records.
    """
    records = list(caplog.get_records("setup")) + list(caplog.records)
    return [
        record
        for record in records
        if record.name == LOGGER_NAME and record.levelno == logging.WARNING
    ]


@pytest.fixture(autouse=True)
def capture_middleware_logs(caplog):
    """Capture the middleware's own log records at WARNING and above."""
    caplog.set_level(logging.WARNING, logger=LOGGER_NAME)


@pytest.fixture
def inner():
    """Return the recording inner application."""
    return RecordingApp()


@pytest.fixture
def send():
    """Return the recording ASGI send."""
    return RecordingSend()


@pytest.fixture
def middleware(inner):
    """Return the middleware under test, wrapping the recording inner app."""
    return BodySizeLimitMiddleware(inner, max_bytes=MAX_BYTES)


@pytest.mark.asyncio
class TestCallOnANonHttpScope:
    """A scope the cap does not apply to is handed straight on.

    A websocket or lifespan scope has no request body to bound, and a lifespan
    scope that never reached the application would leave the app unstarted.
    """

    @pytest.fixture
    def lifespan_scope(self):
        return {"type": "lifespan"}

    async def test_invokes_the_inner_app(self, middleware, inner, send, lifespan_scope):
        await middleware(lifespan_scope, QueuedReceive(), send)

        assert inner.scopes == [lifespan_scope]

    async def test_passes_the_original_receive_through(
        self, middleware, inner, send, lifespan_scope
    ):
        receive = QueuedReceive()

        await middleware(lifespan_scope, receive, send)

        assert inner.receives[0] is receive

    async def test_sends_no_response_of_its_own(self, middleware, send, lifespan_scope):
        await middleware(lifespan_scope, QueuedReceive(), send)

        assert send.messages == []


@pytest.mark.asyncio
class TestCallWithAnOversizeContentLength:
    """A declaration above the cap is refused without reading the body."""

    @pytest.fixture
    def receive(self):
        return QueuedReceive(body_message(MAX_BYTES + 1))

    @pytest_asyncio.fixture(autouse=True)
    async def refused(self, middleware, receive, send):
        await middleware(http_scope(str(MAX_BYTES + 1).encode()), receive, send)

    async def test_responds_with_413(self, send):
        assert send.status == 413

    async def test_does_not_invoke_the_inner_app(self, inner):
        assert inner.was_called is False

    async def test_does_not_read_the_body(self, receive):
        assert receive.call_count == 0

    async def test_body_states_the_cap_in_the_detail_field(self, send):
        assert json.loads(send.body) == {
            "detail": (
                f"Request body exceeds the maximum of {MAX_BYTES} bytes "
                "accepted by this service."
            )
        }

    async def test_declares_a_json_content_type(self, send):
        assert send.headers[b"content-type"] == b"application/json"

    async def test_declares_the_length_of_the_body_it_sent(self, send):
        assert send.headers[b"content-length"] == str(len(send.body)).encode()

    async def test_asks_the_server_to_close_the_connection(self, send):
        # The declared body is never read off the wire on this path, so the
        # connection cannot be reused; the header is what makes the server
        # close it rather than let the client pipeline onto a desynchronized
        # connection.
        assert send.headers[b"connection"] == b"close"

    async def test_logs_one_warning(self, caplog):
        assert len(refusals(caplog)) == 1


@pytest.mark.asyncio
class TestCallWithAnAcceptableContentLength:
    """A declaration at or under the cap passes through untouched.

    Nothing is wrapped and nothing is read here: the declaration is
    authoritative (the ASGI server frames the body by it), so the request
    reaches the inner application exactly as it arrived.
    """

    @pytest.fixture
    def receive(self):
        # The queued body is exactly what the UNDER_CAP declaration announces,
        # as the framing the class docstring describes would produce.
        return QueuedReceive(body_message(UNDER_CAP_BYTES))

    async def test_invokes_the_inner_app(self, middleware, inner, receive, send):
        await middleware(http_scope(UNDER_CAP), receive, send)

        assert inner.was_called is True

    async def test_passes_the_original_receive_unwrapped(
        self, middleware, inner, receive, send
    ):
        await middleware(http_scope(UNDER_CAP), receive, send)

        assert inner.receives[0] is receive

    async def test_accepts_a_declaration_of_exactly_the_cap(
        self, middleware, inner, send
    ):
        receive = QueuedReceive(body_message(MAX_BYTES))

        await middleware(http_scope(str(MAX_BYTES).encode()), receive, send)

        assert inner.was_called is True

    async def test_logs_no_warning(self, middleware, receive, send, caplog):
        await middleware(http_scope(UNDER_CAP), receive, send)

        assert refusals(caplog) == []


@pytest.mark.asyncio
class TestCallWithAnOversizeStreamedBody:
    """Without a usable declaration the body is counted as it arrives."""

    @pytest.fixture
    def receive(self):
        # Three chunks of 6 bytes against a cap of 10: the second crosses it.
        return QueuedReceive(
            body_message(6, more_body=True),
            body_message(6, more_body=True),
            body_message(6, more_body=False),
        )

    @pytest_asyncio.fixture(autouse=True)
    async def refused(self, middleware, receive, send):
        await middleware(http_scope(), receive, send)

    async def test_responds_with_413(self, send):
        assert send.status == 413

    async def test_does_not_invoke_the_inner_app(self, inner):
        assert inner.was_called is False

    async def test_stops_reading_at_the_chunk_that_crosses_the_cap(self, receive):
        assert receive.call_count == 2

    async def test_asks_the_server_to_close_the_connection(self, send):
        # The rest of the body is left unread on this path too -- the drain
        # stops at the chunk that crosses the cap -- so the connection is just
        # as unusable as on the declared-length path and must be closed.
        assert send.headers[b"connection"] == b"close"

    async def test_logs_one_warning(self, caplog):
        assert len(refusals(caplog)) == 1


@pytest.mark.asyncio
class TestCallWithAnAcceptableStreamedBody:
    """A streamed body within the cap is replayed to the inner application."""

    @pytest.fixture
    def messages(self):
        return [
            body_message(4, more_body=True),
            body_message(4, more_body=False),
        ]

    @pytest.fixture
    def receive(self, messages):
        return QueuedReceive(*messages)

    async def test_invokes_the_inner_app(self, middleware, inner, receive, send):
        await middleware(http_scope(), receive, send)

        assert inner.was_called is True

    async def test_replays_the_drained_messages_in_order(
        self, middleware, inner, receive, send, messages
    ):
        await middleware(http_scope(), receive, send)

        assert inner.received == messages

    async def test_delegates_to_the_real_receive_past_the_replay(self, receive, send):
        inner = RecordingApp(trailing_reads=1)

        await BodySizeLimitMiddleware(inner, max_bytes=MAX_BYTES)(
            http_scope(), receive, send
        )

        assert inner.trailing == [DISCONNECT]

    async def test_accepts_a_body_of_exactly_the_cap(self, middleware, inner, send):
        receive = QueuedReceive(body_message(MAX_BYTES))

        await middleware(http_scope(), receive, send)

        assert inner.was_called is True

    async def test_logs_no_warning(self, middleware, receive, send, caplog):
        await middleware(http_scope(), receive, send)

        assert refusals(caplog) == []


@pytest.mark.asyncio
class TestCallWhenTheClientDisconnectsMidBody:
    """A disconnect ends the drain and is replayed with the body.

    The inner application has to see the peer going away, or it waits on a
    body message that will never come.
    """

    @pytest.fixture
    def messages(self):
        return [body_message(4, more_body=True), DISCONNECT]

    @pytest.fixture
    def receive(self, messages):
        return QueuedReceive(*messages)

    async def test_invokes_the_inner_app(self, middleware, inner, receive, send):
        await middleware(http_scope(), receive, send)

        assert inner.was_called is True

    async def test_replays_the_disconnect_after_the_body(
        self, middleware, inner, receive, send, messages
    ):
        await middleware(http_scope(), receive, send)

        assert inner.received == messages


@pytest.mark.asyncio
class TestCallWithAnUntrustworthyContentLength:
    """A declaration that cannot be trusted routes to the counting branch.

    Each declaration below claims a size inside the cap, so trusting it would
    admit an oversize body; each is nevertheless honoured as no declaration at
    all, which means a body within the cap still gets through.
    """

    @pytest.mark.parametrize(
        "declarations", UNTRUSTWORTHY_DECLARATIONS, ids=UNTRUSTWORTHY_IDS
    )
    async def test_refuses_an_oversize_body(self, middleware, send, declarations):
        receive = QueuedReceive(body_message(MAX_BYTES + 1))

        await middleware(http_scope(*declarations), receive, send)

        assert send.status == 413

    @pytest.mark.parametrize(
        "declarations", UNTRUSTWORTHY_DECLARATIONS, ids=UNTRUSTWORTHY_IDS
    )
    async def test_accepts_a_body_within_the_cap(
        self, middleware, inner, send, declarations
    ):
        receive = QueuedReceive(body_message(MAX_BYTES))

        await middleware(http_scope(*declarations), receive, send)

        assert inner.was_called is True
