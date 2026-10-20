# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Unit tests for REST routes."""

import json
from pathlib import Path

import pytest
import yaml
from sse_starlette import sse

from app.config import settings as _settings_module
from app.lib.auth import authenticate
from app.lib.rabbitmq import DurableQueueConsumer, capacity_guard, rabbitmq_connector
from app.lib.rabbitmq.capacity_guard import CapacityGuard
from app.lib.rabbitmq.rabbitmq_base import (
    RECONNECT_TOKEN_MAX_LENGTH,
    RECONNECT_TOKEN_PATTERN,
)
from app.rest import routes
from app.rest.routes import CHANNEL_POOL_EXHAUSTED_RESPONSE
from app.rest.rest_server import rest_server
from notification_common.errors.errors import (
    AuthenticationError,
    ChannelNotAvailableError,
)
from notification_common.rabbitmq.rabbitmq_connector import RabbitMQConnector
from tests.fixtures.queue_id_bounds import (
    MALFORMED_QUEUE_IDS,
    MALFORMED_QUEUE_ID_IDS,
    MALFORMED_RECONNECT_TOKENS,
    MALFORMED_RECONNECT_TOKEN_IDS,
    MAX_LENGTH_RECONNECT_TOKEN,
    MAX_QUEUE_ID_LENGTH,
)

METRICS_PATH = "/api/v1beta/metrics/channel-pool"
AUTH_HEADER = {"Authorization": "Bearer test-token"}

NON_DURABLE_STREAM_PATH = "/api/v1beta/events/stream"
DURABLE_STREAM_PATH = "/api/v1beta/events/stream-durable"
DURABLE_QUEUE_PATH = "/api/v1beta/queues/durable"
HEALTH_PATH = "/api/v1beta/health"

# The full response body of the metrics endpoint. The autoscaler reads these
# keys by name, so renaming one -- or adding a fourth -- is a protocol change.
METRICS_RESPONSE_KEYS = {
    "utilization_percent",
    "channels_in_use",
    "channels_total",
}


def request_non_durable_stream(client):
    """Issue the one request gated on the non-durable pool."""
    return client.get(NON_DURABLE_STREAM_PATH, headers=AUTH_HEADER)


# An id that passes QUEUE_ID_PATTERN, so a request carrying it is turned
# away only by whatever the test under way is actually exercising.
VALID_QUEUE_ID = "q"


def request_durable_stream(client, queue_id=VALID_QUEUE_ID):
    return client.get(
        DURABLE_STREAM_PATH, params={"queue_id": queue_id}, headers=AUTH_HEADER
    )


def request_create_durable_queue(client, queue_id=None):
    body = {"filter_groups": []}
    if queue_id is not None:
        body["queue_id"] = queue_id

    return client.post(DURABLE_QUEUE_PATH, json=body, headers=AUTH_HEADER)


def request_delete_durable_queue(client, queue_id=VALID_QUEUE_ID):
    return client.delete(
        DURABLE_QUEUE_PATH, params={"queue_id": queue_id}, headers=AUTH_HEADER
    )


def request_update_durable_queue(client, queue_id=VALID_QUEUE_ID):
    return client.patch(
        DURABLE_QUEUE_PATH,
        json={
            "queue_id": queue_id,
            "current_filter_groups": [],
            "new_filter_groups": [],
        },
        headers=AUTH_HEADER,
    )


def request_non_durable_stream_with_token(client, reconnect_token):
    """Pass the reconnect token the way a manually reconnecting client does."""
    return client.get(
        NON_DURABLE_STREAM_PATH,
        params={"reconnect_token": reconnect_token},
        headers=AUTH_HEADER,
    )


def request_non_durable_stream_with_last_event_id(client, reconnect_token):
    """
    Pass the same token the way a browser's SSE reconnect does.

    The value is encoded latin-1 and sent as BYTES rather than as ``str``.
    httpx refuses to encode a non-ASCII header value at all, while the wire
    format is latin-1 and Starlette decodes it as such -- so a ``str`` here
    could not express the non-ASCII header a real client genuinely can
    send, which is precisely the case the pattern exists to reject.
    """
    return client.get(
        NON_DURABLE_STREAM_PATH,
        headers={**AUTH_HEADER, "Last-Event-ID": reconnect_token.encode("latin-1")},
    )


# Every route gated on the durable pool. Keyed by name so a parametrized
# test reports which route drifted.
DURABLE_ROUTE_REQUESTS = {
    "stream_durable": request_durable_stream,
    "create_queue": request_create_durable_queue,
    "delete_queue": request_delete_durable_queue,
    "update_queue": request_update_durable_queue,
}


@pytest.fixture
def patch_pool_stats(mocker):
    """
    Return a callable that stubs the connector's channel-pool stats.

    ``channel_pool_metrics()`` reads ``rabbitmq_connector.pool_stats_by_pool()``
    exactly once and selects that report's non-durable entry, so that is the
    read under test. Both connector accessors are stubbed from the one
    ``make_pool_stats_by_pool(...)`` result -- ``pool_stats_by_pool`` with the
    full two-pool dict and the flat ``pool_stats`` with that dict's non-durable
    entry -- so a test that loads the *durable* pool genuinely proves durable
    usage never reaches the response, rather than passing because the read the
    endpoint happens to perform was left unstubbed.

    ``routes.py`` imports the connector singleton directly, so the patch targets
    that object rather than a module path.
    """

    def _patch(stats_by_pool):
        patched = mocker.patch.object(
            rabbitmq_connector,
            "pool_stats_by_pool",
            return_value=stats_by_pool,
        )
        mocker.patch.object(
            rabbitmq_connector,
            "pool_stats",
            return_value=stats_by_pool.get(RabbitMQConnector.NON_DURABLE_POOL_NAME, {}),
        )
        return patched

    return _patch


class TestChannelPoolMetrics:
    """
    Tests for the channel_pool_metrics endpoint, in non-durable STREAM slots.

    Every number below is keyed to the non-durable pool alone, and its capacity
    is that pool's stream capacity -- ``max_pool_size * channel_client_limit``
    (``notification_common/rabbitmq/channel_pool.py:377``). The sizings are
    built with the ``non_durable_client_limit=`` shorthand rather than
    hand-paired ``channel_client_limit`` / ``max_client_capacity`` values, and
    the expected values are asserted as literals with the derivation spelled
    out, so no test re-implements the endpoint's own formula.
    """

    @pytest.fixture(autouse=True)
    def _no_reserve(self, mocker):
        """Default to zero reserve so basic tests verify core logic."""
        mocker.patch.object(
            _settings_module.settings,
            "readiness_min_available_channels",
            0,
        )

    @pytest.fixture
    def stream_unit_response(
        self, client, mocker, patch_pool_stats, make_pool_stats_by_pool
    ):
        """
        A response produced by a non-trivial stream-unit state.

        Client limit 8 with a reserve of 20, so the protocol assertions below
        run on the shipped packing-aware code path rather than on the
        limit-1 special case where streams and channels coincide.
        """
        mocker.patch.object(
            _settings_module.settings,
            "readiness_min_available_channels",
            20,
        )
        patch_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=8,
                non_durable_clients=390,
            )
        )

        return client.get(METRICS_PATH)

    def test_client_limit_1_matches_the_pre_packing_values(
        self, client, patch_pool_stats, make_pool_stats_by_pool
    ):
        """
        At limit 1 the stream-keyed surface reports exactly what it used to.

        One client per channel means a stream slot *is* a channel, so re-keying
        the surface to stream units must not move a single number at today's
        default sizing. This is the numeric-equivalence pin for that: 100 pool
        x 1 client = 100 stream slots, 75 in use.
        """
        patch_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=1,
                non_durable_clients=75,
            )
        )

        response = client.get(METRICS_PATH)

        assert response.status_code == 200
        data = response.json()
        assert data["utilization_percent"] == 75
        assert data["channels_in_use"] == 75
        assert data["channels_total"] == 100

    def test_reports_stream_capacity_at_client_limit_8(
        self, client, patch_pool_stats, make_pool_stats_by_pool
    ):
        """
        Packing 8 clients per channel multiplies the reported capacity.

        The load-bearing assertion is ``channels_total == 800`` (100 pool x 8
        clients), never 100: capacity must be counted in the same stream unit
        as usage. Any channel-keyed implementation fails here -- one that kept
        a channel-unit capacity against stream-unit usage would report 100%
        utilization at half load and hold the autoscaler at max replicas
        forever, which is the packing-defeats-autoscaling failure this Task
        exists to prevent.
        """
        patch_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=8,
                non_durable_clients=400,
            )
        )

        response = client.get(METRICS_PATH)

        assert response.status_code == 200
        data = response.json()
        assert data["channels_in_use"] == 400
        assert data["channels_total"] == 800  # 100 pool x 8 clients
        assert data["utilization_percent"] == 50  # 400 / 800

    def test_reports_stream_capacity_at_client_limit_16(
        self, client, patch_pool_stats, make_pool_stats_by_pool
    ):
        """
        The same keying holds at limit 16, well past the pre-packing sizing.

        ``ChannelPoolMetricsResponse`` imposes no *binding* magnitude ceiling on
        either count -- both carry only a non-binding ``le=_INT32_MAX`` sentinel
        (added for the OWASP integer-limit rule) plus ``ge=0`` (pinned in
        ``tests/app/rest/models/test_channel_pool_metrics.py``); the old finite
        ``le=10000`` bound is gone, so any real-world count stays effectively
        unclamped. The status assertion stays because that is the property under
        test: a re-introduced binding bound would reach the autoscaler as a
        non-200 rather than as a wrong number, so a body-only assertion would
        fail with a confusing ``KeyError`` instead of naming the regression.
        """
        patch_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=16,
                non_durable_clients=800,
            )
        )

        response = client.get(METRICS_PATH)

        assert response.status_code == 200
        data = response.json()
        assert data["channels_in_use"] == 800
        assert data["channels_total"] == 1600  # 100 pool x 16 clients
        assert data["utilization_percent"] == 50  # 800 / 1600

    def test_response_contains_exactly_the_autoscaler_fields(
        self, stream_unit_response
    ):
        """
        The response shape is the autoscaler's protocol and must not drift.

        The three names stay wrong-by-noun on purpose: they now carry stream
        slots, not channels. The scaler and the service skew during rollout
        (the service ships first; the scaler is retuned in Epic 6), so renaming
        a key here would break the deployed scaler mid-rollout. The rename is
        deferred to ticket ``b.3c9``'s observability work. No rename, no fourth
        field.
        """
        data = stream_unit_response.json()

        assert set(data) == METRICS_RESPONSE_KEYS
        # The pin runs on a real stream-unit body: 100 pool x 8 clients, less
        # a reserve of 20 stream slots.
        assert data["channels_total"] == 780

    def test_response_values_are_json_integers(self, stream_unit_response):
        """
        All three values stay bare JSON integers.

        A unit suffix or a stringified value would be a silent protocol break:
        the autoscaler parses these as numbers.
        """
        data = stream_unit_response.json()

        assert [type(data[key]) for key in sorted(METRICS_RESPONSE_KEYS)] == [
            int,
            int,
            int,
        ]

    def test_ignores_durable_capacity(
        self, client, patch_pool_stats, make_pool_stats_by_pool
    ):
        """
        The durable pool's capacity never enters ``channels_total``.

        A durable pool sized to 16 clients per channel (1600 stream slots)
        alongside a non-durable pool at limit 1 still reports 100: not 1600
        (read the wrong pool) and not 1700 (sum the pools). The durable pool
        has its own capacity budget and its own capacity surface, so mixing it
        in here would tell the autoscaler there is non-durable headroom that
        does not exist.
        """
        patch_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=1,
                durable_client_limit=16,
            )
        )

        response = client.get(METRICS_PATH)

        assert response.json()["channels_total"] == 100  # 100 pool x 1 client

    def test_excludes_durable_only_usage(
        self, client, patch_pool_stats, make_pool_stats_by_pool
    ):
        """
        Load confined to the durable pool leaves this surface at zero.

        Decided behavior change, not drift: under Task 1's Cross-Task contract
        (``t2.tj2.nm.53``) durable checkouts no longer consume the non-durable
        surface's slots, so 40 durable clients report 0% here. The durable pool
        is governed by its own guard and its own capacity budget; letting its
        usage move the non-durable scaling signal would scale the service on
        load the non-durable pool is not carrying.
        """
        patch_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=1,
                non_durable_clients=0,
                durable_clients=40,
            )
        )

        response = client.get(METRICS_PATH)

        assert response.status_code == 200
        data = response.json()
        assert data["channels_in_use"] == 0
        assert data["utilization_percent"] == 0
        assert data["channels_total"] == 100

    def test_excludes_durable_usage_from_a_mixed_load(
        self, client, patch_pool_stats, make_pool_stats_by_pool
    ):
        """
        Usage is the non-durable count alone, never the sum across pools.

        50 non-durable plus 25 durable clients reports 50, not 75 -- the
        decided durable-checkout exclusion from Task 1's Cross-Task contract
        (``t2.tj2.nm.53``).
        """
        patch_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=1,
                non_durable_clients=50,
                durable_clients=25,
            )
        )

        response = client.get(METRICS_PATH)

        data = response.json()
        assert data["channels_in_use"] == 50
        assert data["utilization_percent"] == 50  # 50 / 100

    def test_handles_zero_capacity(
        self, client, patch_pool_stats, make_pool_stats_by_pool
    ):
        """A pool reporting no capacity is served, not divided by zero."""
        patch_pool_stats(
            make_pool_stats_by_pool(
                non_durable={"max_client_capacity": 0},
                non_durable_clients=0,
            )
        )

        response = client.get(METRICS_PATH)

        assert response.status_code == 200
        data = response.json()
        assert data["utilization_percent"] == 0

    def test_handles_missing_stats_fields(self, client, patch_pool_stats):
        """
        An empty per-pool report degrades to zero usage, not an error.

        With no non-durable entry to read, capacity falls to the raw floor of
        1 and reports ``channels_total == 1`` after a zero-reserve subtraction.
        The endpoint never raises: a 500 would drop the pod out of the
        autoscaler's view entirely.
        """
        patch_pool_stats({})

        response = client.get(METRICS_PATH)

        assert response.status_code == 200
        data = response.json()
        assert data["utilization_percent"] == 0
        assert data["channels_in_use"] == 0
        assert data["channels_total"] == 1

    def test_absent_durable_pool_entry_is_irrelevant(
        self, client, patch_pool_stats, make_pool_stats_by_pool
    ):
        """
        A report carrying only the non-durable pool reads exactly the same.

        The connector omits the durable entry until that pool is created. Now
        that this surface keys on the non-durable pool alone, the durable
        entry's presence changes nothing -- these are the same numbers the
        limit-8 table above reports with both pools present.
        """
        both_pools = make_pool_stats_by_pool(
            non_durable_client_limit=8,
            non_durable_clients=400,
        )
        patch_pool_stats(
            {
                RabbitMQConnector.NON_DURABLE_POOL_NAME: both_pools[
                    RabbitMQConnector.NON_DURABLE_POOL_NAME
                ]
            }
        )

        response = client.get(METRICS_PATH)

        assert response.status_code == 200
        data = response.json()
        assert data["utilization_percent"] == 50
        assert data["channels_in_use"] == 400
        assert data["channels_total"] == 800  # 100 pool x 8 clients

    def test_clamps_negative_client_count_to_zero(
        self, client, patch_pool_stats, make_pool_stats_by_pool
    ):
        """
        A negative client count reports as zero usage.

        The pool can transiently report a negative count under a race (e.g. a
        double-return of a channel), and a negative gauge would be rejected by
        the response model's ``ge=0`` bound as a 500.
        """
        patch_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=1,
                non_durable_clients=-88,
            )
        )

        response = client.get(METRICS_PATH)

        assert response.status_code == 200
        data = response.json()
        assert data["utilization_percent"] == 0
        assert data["channels_in_use"] == 0
        assert data["channels_total"] == 100

    def test_durable_usage_does_not_offset_a_negative_non_durable_count(
        self, client, patch_pool_stats, make_pool_stats_by_pool
    ):
        """
        The clamp is applied to the non-durable count, with nothing summed in.

        -5 non-durable clients alongside 50 durable ones reports 0, not 45:
        the durable count is not part of this surface at all under Task 1's
        Cross-Task contract (``t2.tj2.nm.53``), so there is no sum left for it
        to lift above zero. Decided change, not drift.
        """
        patch_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=1,
                non_durable_clients=-5,
                durable_clients=50,
            )
        )

        response = client.get(METRICS_PATH)

        assert response.json()["channels_in_use"] == 0

    def test_reports_100_percent_when_streams_exactly_fill_capacity(
        self, client, patch_pool_stats, make_pool_stats_by_pool
    ):
        """Exactly full is 100%, at a packed sizing: 800 of 100 pool x 8."""
        patch_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=8,
                non_durable_clients=800,
            )
        )

        response = client.get(METRICS_PATH)

        data = response.json()
        assert data["utilization_percent"] == 100
        assert data["channels_in_use"] == 800
        assert data["channels_total"] == 800

    def test_beyond_capacity_clamps_utilization_but_not_channels_in_use(
        self, client, patch_pool_stats, make_pool_stats_by_pool
    ):
        """
        Overload clamps the ratio but reports the raw stream count.

        ``utilization_percent`` saturates at 100 because it is a percentage,
        while ``channels_in_use`` is deliberately left effectively unclamped
        (its only maximum is the non-binding ``_INT32_MAX`` sentinel) and may
        exceed ``channels_total``: clamping it would hide the size of a real
        overload from the operator-visible gauge. Past capacity the surface
        degrades and never raises -- 900 streams against 800 of capacity is
        still a 200.
        """
        patch_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=8,
                non_durable_clients=900,
            )
        )

        response = client.get(METRICS_PATH)

        assert response.status_code == 200
        data = response.json()
        assert data["utilization_percent"] == 100
        assert data["channels_in_use"] == 900
        assert data["channels_total"] == 800  # 100 pool x 8 clients

    def test_serves_an_in_use_count_at_operator_cap_scale(
        self, client, patch_pool_stats, make_pool_stats_by_pool
    ):
        """
        An in-use count far past the retired ``le=10000`` bound is still a 200.

        The scenario is an operator who raised the absolute memory cap
        (``max_non_durable_streams_per_pod``) to 20,000 on a pod whose packing
        product is only 100 stream slots: under two-tier admission the overflow
        band admits streams past the scaling threshold up to that cap, so
        ``channels_in_use`` (the pool's ``total_client_count``) legitimately
        reaches 20,000. The field's only maximum is the non-binding
        ``_INT32_MAX`` sentinel (added for the OWASP integer-limit rule), which
        sits far above any operator cap, so 20,000 stays effectively unclamped
        and a 200 -- no finite *binding* bound is safe here, since one would
        make the handler's own ``ChannelPoolMetricsResponse(...)`` raise a
        ``ValidationError`` that ``error_handling_middleware`` maps to a **400**,
        dropping the pod out of the autoscaler's fleet view precisely when it is
        most loaded. Hence the explicit status assertion: such a failure returns
        a response rather than raising, so a body-only test would fail with a
        bare ``KeyError``.

        Derivation: 100 ``max_pool_size`` x 1 client = 100 stream slots of
        capacity, zero reserve, so ``channels_total == 100``;
        ``channels_in_use == 20000`` is reported raw, unclamped to capacity, so
        the operator sees the true size of the overflow; ``utilization_percent``
        saturates at its own surviving ``le=100``.
        """
        patch_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=1,
                non_durable_clients=20000,
            )
        )

        response = client.get(METRICS_PATH)

        assert response.status_code == 200
        data = response.json()
        assert data["channels_in_use"] == 20000
        assert data["channels_total"] == 100  # 100 pool x 1 client
        assert data["utilization_percent"] == 100

    def test_serves_a_capacity_at_operator_cap_scale(
        self, client, patch_pool_stats, make_pool_stats_by_pool
    ):
        """
        A capacity past the retired bound is a 200 too, by the other route.

        ``channels_total`` reaches cap scale independently of the overflow
        band: it is the pool's ``max_client_capacity``, the
        ``max_pool_size x channel_client_limit`` packing product, and both
        factors are operator-settable ints with no maximum. A pod sized to a
        1,000-channel pool packing 16 clients each carries 16,000 stream slots
        of ordinary, non-overflow capacity -- past the old 10,000 ceiling at
        half load. Status is asserted explicitly for the same reason as above:
        the regression surfaces as a 400 response, not an exception.

        Derivation: 1000 pool x 16 clients = 16,000 stream slots, zero reserve;
        8,000 in use is 8000 * 100 // 16000 = 50%.
        """
        patch_pool_stats(
            make_pool_stats_by_pool(
                non_durable={"max_pool_size": 1000},
                non_durable_client_limit=16,
                non_durable_clients=8000,
            )
        )

        response = client.get(METRICS_PATH)

        assert response.status_code == 200
        data = response.json()
        assert data["channels_total"] == 16000  # 1000 pool x 16 clients
        assert data["channels_in_use"] == 8000
        assert data["utilization_percent"] == 50  # 8000 / 16000


class TestChannelPoolMetricsReserve:
    """
    The readiness reserve, subtracted from ``channels_total`` in stream units.

    Subtracting the reserve from a stream capacity is the mixed-units fix: the
    reserve is a count of non-durable STREAM slots, so it is subtracted from
    that pool's stream capacity, not from a channel count and not scaled by the
    packing factor.

    This endpoint and the gRPC readiness servicer read the SAME setting --
    ``readiness_min_available_channels`` -- in that SAME unit
    (``t3.tj2.nm.nv.r7``); no second setting was introduced, and the name is
    kept despite now reading wrong-by-noun. Every test patches the setting
    explicitly: its default value is pinned by Task 2 in
    ``tests/app/config/test_settings.py``, and a developer ``.env`` can move
    the process-global default out from under this file.
    """

    @pytest.fixture
    def reserve(self, mocker):
        """Set the readiness reserve, in non-durable stream slots."""

        def _set(stream_slots):
            mocker.patch.object(
                _settings_module.settings,
                "readiness_min_available_channels",
                stream_slots,
            )

        return _set

    def test_reserve_is_invisible_at_client_limit_1(
        self, client, reserve, patch_pool_stats, make_pool_stats_by_pool
    ):
        """
        At limit 1 the reserve arithmetic is byte-for-byte what it always was.

        100 stream slots less a reserve of 10 is 90, exactly as when both were
        read as channels. That equivalence is why the limit-8 and limit-16 rows
        below are the load-bearing ones: no limit-1 test can tell a stream-unit
        reserve from a channel-unit one.
        """
        reserve(10)
        patch_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=1,
                non_durable_clients=70,
            )
        )

        response = client.get(METRICS_PATH)

        data = response.json()
        assert data["channels_total"] == 90
        assert data["channels_in_use"] == 70

    def test_subtracts_a_flat_stream_reserve_at_client_limit_8(
        self, client, reserve, patch_pool_stats, make_pool_stats_by_pool
    ):
        """
        800 stream slots less a reserve of 20 stream slots is 780.

        The two wrong answers this rejects:

        * ``80`` -- capacity left in channels (100) with the reserve taken off
          it, the mixed-units bug read the other way round.
        * ``640`` -- ``800 - (20 x 8)``, the reserve wrongly scaled by the
          packing factor, which would reserve eight times the headroom the
          operator asked for.

        The pre-Task code already answers 780 here, because the combined view
        sourced capacity from the non-durable entry and the reserve keeps its
        value under the decided option (b). This row is therefore a forward
        regression guard against a future channel-keyed or limit-scaled
        reserve, not a pin on a pre-Task defect.
        """
        reserve(20)
        patch_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=8,
                non_durable_clients=390,
            )
        )

        response = client.get(METRICS_PATH)

        data = response.json()
        assert data["channels_total"] == 780  # (100 pool x 8) - 20
        assert data["utilization_percent"] == 50  # 39000 // 780

    def test_subtracts_a_flat_stream_reserve_at_client_limit_16(
        self, client, reserve, patch_pool_stats, make_pool_stats_by_pool
    ):
        """
        1600 stream slots less a reserve of 20 stream slots is 1580.

        The utilization discriminates the subtraction as well as the unit:
        39500 // 1580 is 25, while the same load against an un-reserved 1600
        would read 24.
        """
        reserve(20)
        patch_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=16,
                non_durable_clients=395,
            )
        )

        response = client.get(METRICS_PATH)

        assert response.status_code == 200
        data = response.json()
        assert data["channels_total"] == 1580  # (100 pool x 16) - 20
        assert data["utilization_percent"] == 25  # 39500 // 1580

    @pytest.mark.parametrize(
        "client_limit, expected_total",
        [
            (1, 0),  # (100 pool x 1) - 200, floored at 0
            (8, 600),  # (100 pool x 8) - 200
        ],
        ids=["limit_1_exhausted_by_reserve", "limit_8_leaves_600"],
    )
    def test_reserve_is_a_flat_stream_count_not_scaled_by_the_limit(
        self,
        client,
        reserve,
        patch_pool_stats,
        make_pool_stats_by_pool,
        client_limit,
        expected_total,
    ):
        """
        One reserve value means one number of stream slots at every sizing.

        The same reserve of 200 wipes out a limit-1 pool's 100 slots and leaves
        600 of a limit-8 pool's 800. A reserve scaled by the packing factor
        would take 200 x 8 off the limit-8 row and floor it at 0, reporting no
        usable capacity on a pool that has 600 slots free and holding the
        autoscaler at 100% forever.
        """
        reserve(200)
        patch_pool_stats(make_pool_stats_by_pool(non_durable_client_limit=client_limit))

        response = client.get(METRICS_PATH)

        assert response.json()["channels_total"] == expected_total

    def test_channels_total_floors_at_zero_and_the_divisor_at_one(
        self, client, reserve, patch_pool_stats, make_pool_stats_by_pool
    ):
        """
        A reserve past capacity reports no usable capacity, not a negative one.

        ``channels_total`` floors at 0 (the response model rejects a negative),
        the utilization divisor separately floors at 1, and the ratio clamps at
        100 -- so any usage at all against no usable capacity reads as fully
        utilized rather than raising.
        """
        reserve(200)
        patch_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=1,
                non_durable_clients=5,
            )
        )

        response = client.get(METRICS_PATH)

        assert response.status_code == 200
        data = response.json()
        assert data["channels_total"] == 0
        assert data["utilization_percent"] == 100

    def test_zero_in_use_with_reserve(
        self, client, reserve, patch_pool_stats, make_pool_stats_by_pool
    ):
        """An idle packed pool reports its reserved capacity at 0% used."""
        reserve(20)
        patch_pool_stats(
            make_pool_stats_by_pool(
                non_durable_client_limit=8,
                non_durable_clients=0,
            )
        )

        response = client.get(METRICS_PATH)

        data = response.json()
        assert data["channels_total"] == 780  # (100 pool x 8) - 20
        assert data["channels_in_use"] == 0
        assert data["utilization_percent"] == 0


class TestHealthCheck:
    """Tests for the health_check endpoint."""

    def test_returns_healthy_status(self, client):
        response = client.get("/api/v1beta/health")

        assert response.status_code == 200
        data = response.json()
        assert data["status"] == "healthy"


@pytest.fixture
def stub_gated_route_bodies(mocker):
    """
    Stub the body of every capacity-gated route so an admitted request
    reaches a real 200 instead of the live RabbitMQ path.

    Admission assertions have to land on a success status, not merely on
    "anything but 503": an unstubbed route raises inside the handler and
    the shared error middleware turns that into a 500, which would satisfy
    a bare ``!= 503`` assertion and hide a gate wired to the wrong surface.

    Returns the stubs by name, so a test can also assert the negative --
    that a refused request reached none of them.
    """

    async def _empty_stream(*_args, **_kwargs):
        async def _no_events():
            # An async generator that yields nothing, so the SSE response
            # completes immediately instead of holding the test client open.
            return
            yield  # pragma: no cover

        return sse.EventSourceResponse(_no_events())

    return {
        "non_durable_events_stream": mocker.patch.object(
            routes, "non_durable_events_stream", side_effect=_empty_stream
        ),
        "durable_events_stream": mocker.patch.object(
            routes, "durable_events_stream", side_effect=_empty_stream
        ),
        "create_queue": mocker.patch.object(
            DurableQueueConsumer, "create_queue", return_value=("q", False)
        ),
        "delete_queue": mocker.patch.object(DurableQueueConsumer, "delete_queue"),
        "update_queue": mocker.patch.object(DurableQueueConsumer, "update_queue"),
    }


class CapacitySurfaceContract:
    """
    Assertions shared by both saturation directions.

    Deliberately not named ``Test*`` so pytest does not collect it on its
    own: the two scenario classes below inherit it, so every test here runs
    once per surface. Each scenario supplies an autouse ``_saturated``
    fixture (which surface rejects), ``rejected_request`` (a call that
    surface must turn away) and ``rejected_route`` (that call's OpenAPI
    coordinates).
    """

    #: Callable taking the test client and issuing the rejected request.
    rejected_request = None

    #: OpenAPI (path, method) of ``rejected_request``, for the documented
    #: 503 response.
    rejected_route = None

    def test_rejected_request_returns_503(self, client):
        assert self.rejected_request(client).status_code == 503

    def test_rejection_body_reports_channel_pool_exhaustion(self, client):
        """
        The 503 body carries the guard's client-facing message.

        ``ChannelNotAvailableError`` propagates to the shared error-handling
        middleware, which renders ``{"detail": <user_message>}``. The
        retry-with-backoff guidance is *not* in that body -- the service
        publishes it in the OpenAPI 503 description instead, which is why
        the test below pins it there rather than here.
        """
        response = self.rejected_request(client)

        assert response.json()["detail"] == ChannelNotAvailableError().user_message

    def test_rejected_route_documents_retry_with_backoff(self):
        """The 503 a client can actually receive is the documented one."""
        path, method = self.rejected_route
        operation = rest_server.openapi()["paths"][path][method]
        description = operation["responses"]["503"]["description"]

        assert description == CHANNEL_POOL_EXHAUSTED_RESPONSE[503]["description"]
        # The guidance a client acts on, spelled out so a rewrite that drops
        # the backoff advice fails here and not only in review.
        assert "retry the request with exponential backoff" in description

    def test_health_not_affected(self, client):
        response = client.get(HEALTH_PATH)

        assert response.status_code == 200

    def test_metrics_not_affected(
        self, client, mocker, patch_pool_stats, make_pool_stats_by_pool
    ):
        """The autoscaler feed keeps serving while the guard rejects clients."""
        mocker.patch.object(
            _settings_module.settings,
            "readiness_min_available_channels",
            0,
        )
        patch_pool_stats(
            make_pool_stats_by_pool(
                non_durable={"max_client_capacity": 100},
                non_durable_clients=100,
            )
        )

        response = client.get(METRICS_PATH)

        assert response.status_code == 200
        assert response.json()["utilization_percent"] == 100


class TestCapacityGuardNonDurableSaturated(CapacitySurfaceContract):
    """
    Non-durable admission refuses: only ``GET /events/stream`` is refused.

    The refusal here is a ``raise_if_at_capacity`` side effect keyed on
    ``durable_pool``, so what this class pins is which SURFACE the
    non-durable admission path gates -- not which tier did the refusing.
    "The pool is saturated" is no longer the right noun for it: non-durable
    admission is three-tier (a report-only pool-capacity target, the
    re-routable shed threshold inside the overflow band, and the
    memory-safe stream cap as the hard bound), and any of the outer two can
    produce this 503. Those tier semantics are pinned against the real
    guard in ``TestStreamMemoryCapRestShed`` and
    ``TestRestStreamsAreUnconditionallyReRoutable`` below.

    The durable routes gate on the durable pool, so a refusing non-durable
    path must not turn a durable client away -- durable consumers no longer
    share the non-durable pool's admission slots.
    """

    rejected_request = staticmethod(request_non_durable_stream)
    rejected_route = ("/api/v1beta/events/stream", "get")

    @pytest.fixture(autouse=True)
    def _saturated(self, capacity_guard_rejections, stub_gated_route_bodies):
        capacity_guard_rejections(non_durable_rejects=True)

    @pytest.mark.parametrize(
        "make_request",
        DURABLE_ROUTE_REQUESTS.values(),
        ids=list(DURABLE_ROUTE_REQUESTS),
    )
    def test_durable_routes_still_admitted(self, client, make_request):
        assert make_request(client).status_code == 200


class TestCapacityGuardDurableExhausted(CapacitySurfaceContract):
    """
    Durable admission refuses: the four durable routes are refused.

    Like the class above this is driven by a ``raise_if_at_capacity`` side
    effect, so it pins the surface split rather than a tier. Durable
    admission does stay single-tier -- gated on the durable pool's own
    stream budget, with no overflow band, no shed threshold and no cap --
    but ``GET /events/stream`` gates on the three-tier non-durable path and
    must keep admitting clients while durable admission refuses.
    """

    rejected_request = staticmethod(request_create_durable_queue)
    rejected_route = ("/api/v1beta/queues/durable", "post")

    @pytest.fixture(autouse=True)
    def _saturated(self, capacity_guard_rejections, stub_gated_route_bodies):
        capacity_guard_rejections(durable_rejects=True)

    @pytest.mark.parametrize(
        "make_request",
        DURABLE_ROUTE_REQUESTS.values(),
        ids=list(DURABLE_ROUTE_REQUESTS),
    )
    def test_durable_routes_return_503(self, client, make_request):
        assert make_request(client).status_code == 503

    def test_non_durable_stream_still_admitted(self, client):
        assert request_non_durable_stream(client).status_code == 200


class TestStreamMemoryCapRestShed:
    """
    The REST row of PRD AC-7: a stream shed by the memory-safe cap 503s.

    The rest of AC-7 is discharged at the RPC level in
    ``tests/integration_tests/grpc/test_memory_safe_stream_cap.py``; REST
    needs only this row because both surfaces call the same
    ``capacity_guard`` and raise the same ``ChannelNotAvailableError``.
    What REST adds is the retry guidance in prose: the 503 description a
    client reads is published in the OpenAPI document, already pinned for
    this exact route by
    ``CapacitySurfaceContract.test_rejected_route_documents_retry_with_backoff``
    above, so it is not re-asserted here.

    Unlike the classes above this drives the REAL guard rather than a
    ``side_effect``: the pool has 100 stream slots and holds 5, so only
    the configured cap of 5 can reject. Overriding the autouse
    ``non_binding_stream_memory_cap`` fixture with a binding cap is the
    opt-in that conftest documents.
    """

    STREAM_CAP = 5

    @pytest.fixture(autouse=True)
    def _pod_at_the_stream_cap(self, mocker, patch_pool_stats, make_pool_stats_by_pool):
        mocker.patch.object(
            _settings_module.settings,
            "max_non_durable_streams_per_pod",
            self.STREAM_CAP,
        )
        patch_pool_stats(make_pool_stats_by_pool(non_durable_clients=self.STREAM_CAP))
        # The suite-wide patch of raise_if_at_capacity stays in place and
        # relays into production admission logic, bound to a guard reading
        # the connector patched just above.
        capacity_guard.raise_if_at_capacity.side_effect = CapacityGuard(
            connector=rabbitmq_connector
        ).raise_if_at_capacity

    def test_stream_at_the_cap_returns_503(self, client):
        assert request_non_durable_stream(client).status_code == 503

    def test_shed_body_reports_channel_pool_exhaustion(self, client):
        response = request_non_durable_stream(client)

        assert response.json()["detail"] == ChannelNotAvailableError().user_message

    def test_durable_route_is_not_shed_by_the_cap(
        self, client, stub_gated_route_bodies
    ):
        # The cap bounds non-durable fan-out only, so the durable pool's
        # own budget still admits -- asserted on a real 200 rather than on
        # "not 503", which a 500 from an unstubbed handler would satisfy.
        assert request_create_durable_queue(client).status_code == 200


#: The stamp Envoy sets on the requests it forwards. REST never reads it;
#: it is sent here only to prove that sending it changes nothing.
PROXY_STAMPED_HEADER = {**AUTH_HEADER, "X-Via-Proxy": "true"}


def request_non_durable_stream_via_proxy(client):
    """Issue the non-durable stream request carrying the proxy stamp."""
    return client.get(NON_DURABLE_STREAM_PATH, headers=PROXY_STAMPED_HEADER)


class TestRestStreamsAreUnconditionallyReRoutable:
    """
    REST/SSE streams are re-routable by class, never by header (SR-3.7).

    ``_check_non_durable_capacity`` passes ``re_routable=True`` as a
    literal. It deliberately does NOT read the ``X-Via-Proxy`` stamp the
    gRPC path keys on, and the reason is a chart fact: the HTTPProxy
    stamps exactly one route, the gRPC ``ConsumeNonDurableEvents``
    (``helm/event-consumer-service/templates/http-proxy.yaml:124``). No
    REST route is stamped, so a header-presence test on this surface would
    read False on every request and misclassify every REST stream as
    PINNED -- admitting it into the band that FR-12 reserves for streams
    scale-out cannot rescue. The classification is correct on its own
    terms too: every REST request is routed afresh and an SSE reconnect
    re-routes by construction, so no REST stream is pinned to this pod.

    Making the gRPC and REST paths "symmetric" by keying REST on the
    header is therefore a plausible-looking regression, which is what the
    stamped row below exists to catch: it is the same request as the
    unstamped one and must get the same 503, not a redundant repeat of it.

    Like ``TestStreamMemoryCapRestShed`` this drives the REAL guard. The
    geometry is conftest opt-in pattern (c): the root
    ``non_binding_stream_memory_cap`` fixture stays in place, so the hard
    bound is 1,000,000 and cannot answer any row here, while
    ``max_re_routable_streams_per_pod`` is patched to 120 -- above the
    default pool's ``max_client_capacity`` of 100, so the clamp leaves it
    alone and the shed line lands genuinely inside the overflow band. A
    pod holding 120 streams is thus past the shed line but nowhere near
    the cap, and every 503 below is the shed tier's doing alone.
    """

    #: Streams held by the pod when the shed tier must refuse. Above the
    #: default pool capacity (100) and far below the non-binding cap.
    SHED_THRESHOLD = 120

    @pytest.fixture(autouse=True)
    def _shed_threshold_inside_the_overflow_band(self, mocker):
        mocker.patch.object(
            _settings_module.settings,
            "max_re_routable_streams_per_pod",
            self.SHED_THRESHOLD,
        )

    @pytest.fixture
    def pod_holding(self, patch_pool_stats, make_pool_stats_by_pool):
        """
        Return a callable putting the non-durable pool at ``streams``
        clients and relaying the suite-wide ``raise_if_at_capacity`` patch
        into production admission logic, bound to a guard that reads the
        connector patched here.
        """

        def _hold(streams):
            patch_pool_stats(make_pool_stats_by_pool(non_durable_clients=streams))
            capacity_guard.raise_if_at_capacity.side_effect = CapacityGuard(
                connector=rabbitmq_connector
            ).raise_if_at_capacity

        return _hold

    def test_unstamped_stream_at_the_shed_threshold_is_shed(self, client, pod_holding):
        """Nothing stamped this request and it is still shed."""
        pod_holding(self.SHED_THRESHOLD)

        assert request_non_durable_stream(client).status_code == 503

    def test_stamped_stream_at_the_shed_threshold_is_shed(self, client, pod_holding):
        """The same request carrying the stamp gets the same answer."""
        pod_holding(self.SHED_THRESHOLD)

        assert request_non_durable_stream_via_proxy(client).status_code == 503

    def test_unstamped_stream_below_the_shed_threshold_is_admitted(
        self, client, pod_holding, stub_gated_route_bodies
    ):
        # Anti-vacuity: one stream below the line the surface admits, so
        # the 503s above are the threshold's doing and not a blanket
        # refusal of REST streams. Asserted on a real 200 rather than on
        # "not 503", per the precedent above.
        pod_holding(self.SHED_THRESHOLD - 1)

        assert request_non_durable_stream(client).status_code == 200

    def test_stamped_stream_below_the_shed_threshold_is_admitted(
        self, client, pod_holding, stub_gated_route_bodies
    ):
        # The stamped row needs its own anti-vacuity partner: without it,
        # an implementation that simply refused any request carrying
        # X-Via-Proxy would satisfy every other row here.
        pod_holding(self.SHED_THRESHOLD - 1)

        assert request_non_durable_stream_via_proxy(client).status_code == 200

    def test_durable_route_is_admitted_at_the_shed_threshold(
        self, client, pod_holding, stub_gated_route_bodies
    ):
        # The shed tier lives on the non-durable path only; durable
        # admission is single-tier on its own pool, which is idle here.
        pod_holding(self.SHED_THRESHOLD)

        assert request_create_durable_queue(client).status_code == 200


# Every route that requires a valid bearer token: the four durable routes
# plus the non-durable stream. Health and the autoscaler metrics feed are
# deliberately absent -- they are in ``AUTH_EXCLUDED_PATHS``.
AUTHENTICATED_ROUTE_REQUESTS = {
    **DURABLE_ROUTE_REQUESTS,
    "stream_non_durable": request_non_durable_stream,
}


@pytest.fixture
def override_authenticate():
    """
    Replace the ``authenticate`` dependency for one test.

    FastAPI captures the callable in ``Depends(authenticate)`` when the
    route is declared, so patching the module attribute would change
    nothing; ``dependency_overrides`` keyed on the same callable is the
    supported seam. Returns a callable taking an optional ``side_effect``
    and yielding the recording stand-in, and clears the override
    afterwards so it cannot leak into another test.
    """

    def _install(side_effect=None):
        calls = []

        async def _authenticate():
            calls.append(True)
            if side_effect is not None:
                raise side_effect

        rest_server.dependency_overrides[authenticate] = _authenticate
        return calls

    yield _install

    rest_server.dependency_overrides.pop(authenticate, None)


class TestAuthenticatedRoutes:
    """
    Every authenticated route validates the caller's token before serving.

    The REST auth middleware resolves the bearer token before the request
    reaches a route, so a token that does not resolve is already refused by
    the time these run. Each route declares the check anyway, as the one
    that would still refuse a request if a real route ever landed in
    ``AUTH_EXCLUDED_PATHS``.
    """

    @pytest.mark.parametrize(
        "make_request",
        AUTHENTICATED_ROUTE_REQUESTS.values(),
        ids=list(AUTHENTICATED_ROUTE_REQUESTS),
    )
    def test_route_runs_the_authentication_dependency(
        self, client, make_request, override_authenticate, stub_gated_route_bodies
    ):
        calls = override_authenticate()

        make_request(client)

        assert len(calls) == 1

    @pytest.mark.parametrize(
        "make_request",
        AUTHENTICATED_ROUTE_REQUESTS.values(),
        ids=list(AUTHENTICATED_ROUTE_REQUESTS),
    )
    def test_rejected_token_returns_401(
        self, client, make_request, override_authenticate, stub_gated_route_bodies
    ):
        override_authenticate(side_effect=AuthenticationError("Token expired"))

        assert make_request(client).status_code == 401

    def test_rejected_token_reports_the_authentication_failure(
        self, client, override_authenticate, stub_gated_route_bodies
    ):
        # The same ``{"detail": <user_message>}`` body the shared error
        # middleware renders for every other client-facing error.
        override_authenticate(side_effect=AuthenticationError("Token expired"))

        response = request_create_durable_queue(client)

        assert response.json()["detail"] == "Token expired"

    def test_health_does_not_authenticate(self, client, override_authenticate):
        # Health is polled by the kubelet, which carries no token.
        calls = override_authenticate()

        client.get(HEALTH_PATH)

        assert calls == []

    def test_metrics_does_not_authenticate(
        self, client, override_authenticate, patch_pool_stats, make_pool_stats_by_pool
    ):
        # The autoscaler feed is scraped in-cluster and carries no token.
        calls = override_authenticate()
        patch_pool_stats(make_pool_stats_by_pool())

        client.get(METRICS_PATH)

        assert calls == []


#: The generated API documentation, and the static OAuth callback shim the
#: Swagger page uses. Both are in ``AUTH_EXCLUDED_PATHS``; neither is a route
#: this service declares, so neither carries the ``authenticate`` dependency
#: and the exclusion list is the only thing that decides whether they serve.
GENERATED_DOCS_PATHS = {
    "swagger": "/docs",
    "swagger_oauth2_redirect": "/docs/oauth2-redirect",
    "openapi": "/openapi.json",
    "redoc": "/redoc",
}


class TestGeneratedDocsPathsServeWithoutAToken:
    """
    The four generated documentation paths answer an unauthenticated GET.

    These are gated by the REST auth middleware alone, which refuses any
    path outside ``AUTH_EXCLUDED_PATHS`` before a route is ever reached. The
    list is passed explicitly to ``create_app``, so this service inherits
    none of the library's defaults and a path missing from it 401s -- which
    is what ``/redoc`` and ``/docs/oauth2-redirect`` did. Browsers reach all
    four with no Authorization header, and none of them exposes anything
    ``/docs`` and ``/openapi.json`` do not already expose unauthenticated:
    ``/redoc`` renders the same document ``/openapi.json`` serves, and
    ``/docs/oauth2-redirect`` is the static OAuth callback shim ``/docs``
    uses.
    """

    @pytest.mark.parametrize(
        "path", GENERATED_DOCS_PATHS.values(), ids=list(GENERATED_DOCS_PATHS)
    )
    def test_serves_without_an_authorization_header(self, client, path):
        assert client.get(path).status_code == 200

    def test_an_authenticated_route_still_refuses_a_tokenless_request(self, client):
        # The control for the four above: the middleware is genuinely
        # refusing tokenless requests, so those 200s are attributable to the
        # exclusion list rather than to authentication being off.
        assert client.get(NON_DURABLE_STREAM_PATH).status_code == 401


class TestNonDurableStreamAdmissionPrecedesAuthentication:
    """
    Capacity is checked before the token, matching the gRPC surface.

    ``Depends(authenticate)`` is declared after
    ``Depends(_check_non_durable_capacity)``, and FastAPI resolves a
    route's dependencies in declaration order, so a saturated pod answers a
    caller whose token resolves with the capacity status that caller
    retries on rather than a 401 it would not.

    The ordering is about the answer, not about the cost: the auth
    middleware has already resolved the token by the time any dependency
    runs, so a refused stream has paid for that resolution either way, and
    a caller whose token does NOT resolve is refused 401 above the route
    entirely.
    """

    def test_a_refused_stream_never_reaches_authentication(
        self, client, capacity_guard_rejections, override_authenticate
    ):
        calls = override_authenticate()
        capacity_guard_rejections(non_durable_rejects=True)

        assert request_non_durable_stream(client).status_code == 503
        assert calls == []

    def test_an_admitted_stream_still_authenticates(
        self,
        client,
        capacity_guard_rejections,
        override_authenticate,
        stub_gated_route_bodies,
    ):
        calls = override_authenticate()
        capacity_guard_rejections()

        assert request_non_durable_stream(client).status_code == 200
        assert len(calls) == 1


class TestAdmissionPrecedesAuthenticationOnEveryDurableRoute:
    """
    The same ordering on the four durable routes, which gate on
    ``_check_durable_capacity`` rather than the non-durable check above.

    FastAPI resolves a route's dependencies in declaration order, and each
    of these declares ``Depends(_check_durable_capacity)`` ahead of
    ``Depends(authenticate)``. Reordering them is a one-line edit that
    changes nothing visible in a passing test suite -- both orderings still
    return an error -- but it turns a saturated pod's answer from the 503 a
    client backs off on into a 401 it would treat as a credential problem
    and not retry.
    """

    @pytest.mark.parametrize(
        "make_request",
        DURABLE_ROUTE_REQUESTS.values(),
        ids=list(DURABLE_ROUTE_REQUESTS),
    )
    def test_a_refused_request_never_reaches_authentication(
        self, client, make_request, capacity_guard_rejections, override_authenticate
    ):
        calls = override_authenticate()
        capacity_guard_rejections(durable_rejects=True)

        make_request(client)

        assert calls == []

    @pytest.mark.parametrize(
        "make_request",
        DURABLE_ROUTE_REQUESTS.values(),
        ids=list(DURABLE_ROUTE_REQUESTS),
    )
    def test_a_refused_request_answers_with_the_capacity_status(
        self, client, make_request, capacity_guard_rejections, override_authenticate
    ):
        override_authenticate()
        capacity_guard_rejections(durable_rejects=True)

        assert make_request(client).status_code == 503

    @pytest.mark.parametrize(
        "make_request",
        DURABLE_ROUTE_REQUESTS.values(),
        ids=list(DURABLE_ROUTE_REQUESTS),
    )
    def test_a_rejected_token_is_still_reported_when_capacity_admits(
        self, client, make_request, capacity_guard_rejections, override_authenticate
    ):
        # The other half of the ordering: admission passing must not let
        # the route skip the token check.
        override_authenticate(side_effect=AuthenticationError("Token expired"))
        capacity_guard_rejections()

        assert make_request(client).status_code == 401

    @pytest.mark.parametrize(
        "make_request",
        DURABLE_ROUTE_REQUESTS.values(),
        ids=list(DURABLE_ROUTE_REQUESTS),
    )
    def test_a_refused_request_does_no_broker_work(
        self,
        client,
        make_request,
        capacity_guard_rejections,
        override_authenticate,
        stub_gated_route_bodies,
    ):
        override_authenticate()
        capacity_guard_rejections(durable_rejects=True)

        make_request(client)

        for stub in stub_gated_route_bodies.values():
            stub.assert_not_called()


# Every REST route that takes a client-supplied queue_id, paired with the
# ``detail[0].loc`` a client reads to find out which parameter it got wrong.
# Both spellings appear: the two query-parameter routes and the two
# request-body routes.
QUEUE_ID_ROUTE_REQUESTS = {
    "stream_durable": (request_durable_stream, ["query", "queue_id"]),
    "create_queue": (request_create_durable_queue, ["body", "queue_id"]),
    "delete_queue": (request_delete_durable_queue, ["query", "queue_id"]),
    "update_queue": (request_update_durable_queue, ["body", "queue_id"]),
}

# The five operations whose OpenAPI 422 is declared by hand, through
# ``_validation_error_response``. Each replaces the generic "Validation
# Error" response FastAPI would have generated.
VALIDATION_ERROR_OPERATIONS = {
    "stream_durable": (DURABLE_STREAM_PATH, "get"),
    "stream_non_durable": (NON_DURABLE_STREAM_PATH, "get"),
    "create_queue": (DURABLE_QUEUE_PATH, "post"),
    "delete_queue": (DURABLE_QUEUE_PATH, "delete"),
    "update_queue": (DURABLE_QUEUE_PATH, "patch"),
}


class TestQueueIdValidatedOnEveryDurableRoute:
    """
    Every REST route taking a client-supplied queue_id applies the shared
    pattern, and reports a violation as a typed 422.

    The byte-budget arithmetic behind ``QUEUE_ID_PATTERN`` only holds if
    ALL of them do -- a single unvalidated surface reintroduces over-long
    broker names, which die building the AMQP frame rather than at the
    broker. The REST half of that enforcement is declarative
    (``pattern=`` / ``max_length=`` on the parameter or model field), so
    what these pin is that the declaration is present on each route and
    reaches the client as the documented status.
    """

    @pytest.mark.parametrize(
        "make_request",
        [entry[0] for entry in QUEUE_ID_ROUTE_REQUESTS.values()],
        ids=list(QUEUE_ID_ROUTE_REQUESTS),
    )
    @pytest.mark.parametrize(
        "queue_id", MALFORMED_QUEUE_IDS, ids=MALFORMED_QUEUE_ID_IDS
    )
    def test_a_malformed_queue_id_returns_422(
        self, client, make_request, queue_id, override_authenticate
    ):
        override_authenticate()

        assert make_request(client, queue_id).status_code == 422

    @pytest.mark.parametrize(
        "make_request, expected_loc",
        QUEUE_ID_ROUTE_REQUESTS.values(),
        ids=list(QUEUE_ID_ROUTE_REQUESTS),
    )
    def test_the_422_body_names_the_queue_id_parameter(
        self, client, make_request, expected_loc, override_authenticate
    ):
        # The typed ``HTTPValidationError`` body, not a bare message: a
        # generated client reads ``loc`` to attribute the failure to the
        # field it sent, which differs by route between query and body.
        override_authenticate()

        response = make_request(client, "bad:id")

        assert response.json()["detail"][0]["loc"] == expected_loc

    @pytest.mark.parametrize(
        "make_request",
        [entry[0] for entry in QUEUE_ID_ROUTE_REQUESTS.values()],
        ids=list(QUEUE_ID_ROUTE_REQUESTS),
    )
    def test_a_maximum_length_queue_id_is_accepted(
        self,
        client,
        make_request,
        override_authenticate,
        stub_gated_route_bodies,
    ):
        # The other half: the bound accepts what it should, so a legal id
        # reaches the handler rather than being turned away one character
        # early.
        override_authenticate()

        response = make_request(client, "a" * MAX_QUEUE_ID_LENGTH)

        assert response.status_code == 200

    def test_a_refused_request_is_not_reported_as_a_bad_queue_id(
        self, client, capacity_guard_rejections, override_authenticate
    ):
        # Dependencies resolve before parameter validation, so a saturated
        # pod answers with the capacity status the client retries on --
        # not a 422 the client would treat as its own bug.
        override_authenticate()
        capacity_guard_rejections(durable_rejects=True)

        assert request_delete_durable_queue(client, "bad:id").status_code == 503

    def test_a_rejected_token_is_not_reported_as_a_bad_queue_id(
        self, client, override_authenticate
    ):
        # Same ordering, one dependency later: an unauthenticated caller is
        # told it is unauthenticated rather than being told how the service
        # validates fields it was never entitled to send.
        override_authenticate(side_effect=AuthenticationError("Token expired"))

        assert request_delete_durable_queue(client, "bad:id").status_code == 401


# A filter group as the query string carries it: a JSON list, which
# ``_parse_new_filter_groups`` / ``_parse_prev_filter_groups`` validate into
# ``FilterGroup`` objects before the route body runs. Two DIFFERENT groups,
# so a test can tell the before-list from the after-list in the call the
# route makes.
PREVIOUS_FILTER_GROUPS_JSON = json.dumps(
    [
        {
            "event_type": "test_event",
            "filters": [{"filter_type": "eq", "resource_id": "before"}],
        }
    ]
)
NEW_FILTER_GROUPS_JSON = json.dumps(
    [
        {
            "event_type": "test_event",
            "filters": [{"filter_type": "eq", "resource_id": "after"}],
        }
    ]
)

RECONNECT_TOKEN = "a-reconnect-token"


def _resource_ids(filter_groups):
    """The resource ids of a parsed filter-group list, or None."""
    if filter_groups is None:
        return None
    return [f.resource_id for group in filter_groups for f in group.filters]


class TestLastEventIdFilterUpdate:
    """
    What the ``Last-Event-ID`` header does with filters on the way in.

    A browser's automatic SSE reconnect replays the request URL it opened
    the stream with, so a stream opened as a fresh connect replays its
    ``filter_groups`` with no ``previous_filter_groups`` -- and passing
    those on would fail as an incomplete filter update. Sending
    ``previous_filter_groups`` is what makes a request an update, so a
    client that sends one asked for an update deliberately and gets one,
    whichever carrier brought the token.

    Read through the ``non_durable_events_stream`` stub because the
    decision is made in the route body, between parameter parsing and the
    handler: what the handler is CALLED WITH is the whole contract.
    """

    @pytest.fixture(autouse=True)
    def authenticated(self, override_authenticate):
        override_authenticate()

    @pytest.fixture
    def handler(self, stub_gated_route_bodies):
        return stub_gated_route_bodies["non_durable_events_stream"]

    @pytest.fixture
    def call_args(self, client, handler):
        """
        Issue a request and hand back ``(filter_groups,
        previous_filter_groups)`` as the route passed them on.
        """

        def _request(*, header=False, query=False, new=False, previous=False):
            params = {}
            headers = dict(AUTH_HEADER)
            if header:
                headers["Last-Event-ID"] = RECONNECT_TOKEN
            if query:
                params["reconnect_token"] = RECONNECT_TOKEN
            if new:
                params["filter_groups"] = NEW_FILTER_GROUPS_JSON
            if previous:
                params["previous_filter_groups"] = PREVIOUS_FILTER_GROUPS_JSON

            response = client.get(
                NON_DURABLE_STREAM_PATH, params=params, headers=headers
            )
            assert response.status_code == 200, response.text

            _request_arg, filter_groups, previous_filter_groups, _token = (
                handler.call_args.args
            )
            return _resource_ids(filter_groups), _resource_ids(previous_filter_groups)

        return _request

    def test_header_with_both_lists_forwards_the_new_filters(self, call_args):
        # The update the header could not express before: both lists are
        # present, so this is a deliberate filter update.
        new, _previous = call_args(header=True, new=True, previous=True)

        assert new == ["after"]

    def test_header_with_both_lists_forwards_the_previous_filters(self, call_args):
        _new, previous = call_args(header=True, new=True, previous=True)

        assert previous == ["before"]

    def test_header_with_only_new_filters_drops_them(self, call_args):
        # The browser replay: the URL a fresh connect was opened with
        # carries its filter_groups and no previous_filter_groups. Passing
        # them on would fail as an incomplete update, so the stream resumes
        # as it was.
        new, _previous = call_args(header=True, new=True)

        assert new is None

    def test_header_alone_forwards_no_filters(self, call_args):
        assert call_args(header=True) == (None, None)

    def test_header_with_only_previous_filters_forwards_them_uncleared(self, call_args):
        # The half-update: a previous list with nothing to update TO. The
        # route does not clear it -- sending one is what makes a request an
        # update, so the shape reaches the handler and `get_instance`
        # rejects it as an incomplete update, exactly as it does when the
        # query carrier brings the same shape. Only the forwarding is this
        # route's contract; the 400 is pinned in the consumer's tests.
        assert call_args(header=True, previous=True) == (None, ["before"])

    def test_the_query_carrier_with_both_lists_is_unchanged(self, call_args):
        # The deprecated carrier's filter-update path is untouched.
        assert call_args(query=True, new=True, previous=True) == (
            ["after"],
            ["before"],
        )

    def test_the_query_carrier_keeps_filters_without_a_previous_list(self, call_args):
        # No clearing on the query path, ever: only a browser replays a URL,
        # and only the header carries a browser's token.
        new, _previous = call_args(query=True, new=True)

        assert new == ["after"]

    def test_filters_alone_are_forwarded_on_a_fresh_connect(self, call_args):
        # The control: with no token at all this is a fresh connect, and its
        # filters must survive whatever the reconnect branch does.
        new, _previous = call_args(new=True)

        assert new == ["after"]


# The two carriers of the reconnect token, paired with the ``detail[0].loc``
# a client reads to find out which one it got wrong.
RECONNECT_TOKEN_CARRIERS = {
    "query": (request_non_durable_stream_with_token, ["query", "reconnect_token"]),
    "last-event-id-header": (
        request_non_durable_stream_with_last_event_id,
        ["header", "last-event-id"],
    ),
}


class TestReconnectTokenConstrainedOnTheNonDurableStream:
    """
    A reconnect token becomes the non-durable queue's broker name, so both
    its length and its alphabet are constrained.

    It arrives two ways -- the ``reconnect_token`` query parameter a
    manually reconnecting client sends, and the ``Last-Event-ID`` header a
    browser sends on its own SSE reconnect -- and both must carry the same
    constraint. An unconstrained header would be an unvalidated way in to
    the same broker queue name.

    The REST half of the enforcement is declarative (``pattern=`` on the
    parameter), matching the imperative ``validate_reconnect_token`` the
    gRPC surface runs, so both surfaces reject the same tokens for the same
    reason.
    """

    @pytest.mark.parametrize(
        "make_request",
        [entry[0] for entry in RECONNECT_TOKEN_CARRIERS.values()],
        ids=list(RECONNECT_TOKEN_CARRIERS),
    )
    @pytest.mark.parametrize(
        "token", MALFORMED_RECONNECT_TOKENS, ids=MALFORMED_RECONNECT_TOKEN_IDS
    )
    def test_a_malformed_token_returns_422(
        self, client, make_request, token, override_authenticate
    ):
        override_authenticate()

        response = make_request(client, token)

        assert response.status_code == 422

    @pytest.mark.parametrize(
        "make_request, expected_loc",
        RECONNECT_TOKEN_CARRIERS.values(),
        ids=list(RECONNECT_TOKEN_CARRIERS),
    )
    @pytest.mark.parametrize(
        "token", MALFORMED_RECONNECT_TOKENS, ids=MALFORMED_RECONNECT_TOKEN_IDS
    )
    def test_the_422_body_names_the_parameter_that_carried_the_token(
        self, client, make_request, expected_loc, token, override_authenticate
    ):
        # A client that sends the token both ways has to be told which one
        # the service objected to.
        override_authenticate()

        response = make_request(client, token)

        assert response.json()["detail"][0]["loc"] == expected_loc

    @pytest.mark.parametrize(
        "make_request",
        [entry[0] for entry in RECONNECT_TOKEN_CARRIERS.values()],
        ids=list(RECONNECT_TOKEN_CARRIERS),
    )
    def test_a_maximum_length_token_reaches_the_handler(
        self, client, make_request, override_authenticate, stub_gated_route_bodies
    ):
        override_authenticate()

        response = make_request(client, MAX_LENGTH_RECONNECT_TOKEN)

        assert response.status_code == 200

    @pytest.mark.parametrize(
        "make_request",
        [entry[0] for entry in RECONNECT_TOKEN_CARRIERS.values()],
        ids=list(RECONNECT_TOKEN_CARRIERS),
    )
    def test_an_unrecognised_token_of_legal_shape_reaches_the_handler(
        self, client, make_request, override_authenticate, stub_gated_route_bodies
    ):
        # The alphabet is deliberately wide: a token this service did not
        # mint but could have has to reach the lookup and fail there as
        # "queue not found", not be turned away here as malformed.
        override_authenticate()

        response = make_request(client, "not-a-token-we-minted:~!")

        assert response.status_code == 200


# The two carriers as the GENERATED SPEC names them. The header's spec name
# is its lowercase wire spelling, which is also the ``detail[0].loc`` the
# behavioural tests above assert -- so the documented name and the name a
# client is told it got wrong are the same string.
RECONNECT_TOKEN_PARAMETERS = {
    "query": ("reconnect_token", "query"),
    "last-event-id-header": ("last-event-id", "header"),
}


def _stream_parameters(spec):
    """The non-durable stream operation's parameters, keyed by spec name."""
    parameters = spec["paths"][NON_DURABLE_STREAM_PATH]["get"]["parameters"]
    return {parameter["name"]: parameter for parameter in parameters}


def _schema_constraint(parameter, key):
    """
    Pull a constraint off a parameter's schema.

    The query carrier is ``str | None``, which FastAPI renders as an ``anyOf``
    of the string option and a null option, so its constraint lives on the
    string branch. The ``Last-Event-ID`` header is a plain ``str`` with a
    ``None`` default (the api-governance OWASP ``no-numeric-ids`` rule needs
    an explicit top-level ``type`` on any ``*-id`` parameter), so its
    constraint sits at the top level. Returned as a list so a constraint that
    went missing and one duplicated onto both branches fail differently.
    """
    schema = parameter["schema"]
    options = schema["anyOf"] if "anyOf" in schema else [schema]
    return [option[key] for option in options if key in option]


class TestReconnectTokenCarriersAreDocumented:
    """
    Both carriers reach the generated spec, declaring the SAME constraint,
    and only one of them is deprecated.

    The query parameter is deprecated because a query string is recorded
    verbatim by proxies, gateways, and access logs along the whole path of
    the request, and this token is the queue's identifier -- so carrying it
    there spreads it to places that do not need it. The header never appears
    in a URL, which is why it is the carrier the deprecation steers clients
    to, and why it must NOT be marked deprecated alongside the parameter it
    replaces.

    Read off ``rest_server.openapi()`` rather than the route signature,
    because the spec is where a client author and a code generator actually
    look: a constraint or a deprecation that never left the source steers
    nobody. That the header is published at all is half the contract -- a
    deprecation pointing at an undocumented replacement leaves a client with
    nothing to migrate to.
    """

    @pytest.mark.parametrize(
        "name, location",
        RECONNECT_TOKEN_PARAMETERS.values(),
        ids=list(RECONNECT_TOKEN_PARAMETERS),
    )
    def test_the_carrier_is_published(self, name, location):
        # Guards every assertion below: a parameter absent from the spec
        # would make the constraint lookups error rather than fail, and an
        # absent HEADER is the specific regression this pins.
        assert name in _stream_parameters(rest_server.openapi())

    @pytest.mark.parametrize(
        "name, location",
        RECONNECT_TOKEN_PARAMETERS.values(),
        ids=list(RECONNECT_TOKEN_PARAMETERS),
    )
    def test_the_carrier_is_published_on_the_right_part_of_the_request(
        self, name, location
    ):
        # A header published as a query parameter would document a way in
        # that does not exist, and hide the one that does.
        assert _stream_parameters(rest_server.openapi())[name]["in"] == location

    @pytest.mark.parametrize(
        "name, location",
        RECONNECT_TOKEN_PARAMETERS.values(),
        ids=list(RECONNECT_TOKEN_PARAMETERS),
    )
    def test_the_carrier_publishes_the_shared_pattern(self, name, location):
        # The declarative half of the enforcement, as a client sees it: the
        # same string the gRPC validator matches against.
        parameter = _stream_parameters(rest_server.openapi())[name]

        assert _schema_constraint(parameter, "pattern") == [RECONNECT_TOKEN_PATTERN]

    @pytest.mark.parametrize(
        "name, location",
        RECONNECT_TOKEN_PARAMETERS.values(),
        ids=list(RECONNECT_TOKEN_PARAMETERS),
    )
    def test_the_carrier_publishes_the_length_bound(self, name, location):
        # The pattern carries the bound too, but a generated client that
        # only reads ``maxLength`` must not have lost it.
        parameter = _stream_parameters(rest_server.openapi())[name]

        assert _schema_constraint(parameter, "maxLength") == [
            RECONNECT_TOKEN_MAX_LENGTH
        ]

    def test_the_two_carriers_publish_the_same_pattern(self):
        # The property that matters more than either value: a client can
        # switch carriers without the service changing its mind about the
        # token. Asserted as "the two agree", so it stays true if the
        # pattern changes and both carriers follow it.
        parameters = _stream_parameters(rest_server.openapi())

        assert _schema_constraint(
            parameters["reconnect_token"], "pattern"
        ) == _schema_constraint(parameters["last-event-id"], "pattern")

    def test_the_query_parameter_is_documented_as_deprecated(self):
        assert _stream_parameters(rest_server.openapi())["reconnect_token"][
            "deprecated"
        ]

    def test_the_header_is_not_documented_as_deprecated(self):
        # It is the carrier the deprecation steers clients TO. Marking it
        # too would tell a client that both ways in are going away.
        assert not _stream_parameters(rest_server.openapi())["last-event-id"].get(
            "deprecated"
        )

    def test_no_other_parameter_on_the_route_is_deprecated(self):
        # Nothing else was swept up: the deprecation names one carrier, not
        # the endpoint.
        parameters = _stream_parameters(rest_server.openapi())

        deprecated = [
            name
            for name, parameter in parameters.items()
            if parameter.get("deprecated")
        ]
        assert deprecated == ["reconnect_token"]


class TestTheGeneratedOpenApiFileCarriesBothCarriers:
    """
    The checked-in ``openapi.yaml`` is the artifact clients are handed, and
    it is generated -- so it is only as current as the last regeneration.

    ``rest_server.openapi()`` is built from the live route objects and
    always agrees with the source by construction; the file does not. A
    route change shipped without ``uv run generate_openapi_yaml`` leaves
    every assertion above passing while the document customers read still
    describes the old surface. That is exactly how a carrier can be missing
    from the published spec without anything failing.
    """

    @staticmethod
    def _parameters():
        openapi_path = Path(__file__).parents[3] / "openapi.yaml"
        return _stream_parameters(yaml.safe_load(openapi_path.read_text()))

    @pytest.mark.parametrize(
        "name, location",
        RECONNECT_TOKEN_PARAMETERS.values(),
        ids=list(RECONNECT_TOKEN_PARAMETERS),
    )
    def test_the_carrier_is_in_the_published_document(self, name, location):
        assert name in self._parameters()

    @pytest.mark.parametrize(
        "name, location",
        RECONNECT_TOKEN_PARAMETERS.values(),
        ids=list(RECONNECT_TOKEN_PARAMETERS),
    )
    def test_the_published_carrier_matches_the_live_spec(self, name, location):
        # Compared whole rather than field by field: any drift between the
        # file and the routes -- a constraint, the deprecation, the
        # description -- is a stale regeneration, and they all mislead a
        # client the same way.
        live = _stream_parameters(rest_server.openapi())[name]

        assert self._parameters()[name] == live

    def test_the_endpoint_itself_is_not_deprecated(self):
        # Deprecating the parameter must not read as deprecating the
        # stream: the endpoint is how a client consumes non-durable events.
        operation = rest_server.openapi()["paths"][NON_DURABLE_STREAM_PATH]["get"]

        assert not operation.get("deprecated")


class TestValidationErrorIsDocumented:
    """
    The 422 a client can actually receive is the documented one, and it
    still carries a body a generated client can parse.

    The hand-written descriptions name the field that failed, which is the
    reason they replace FastAPI's generic "Validation Error". Replacing a
    generated response is easy to do by dropping its ``content`` along the
    way, which would leave the spec saying a 422 happens without saying
    what comes back -- so the ``$ref`` is pinned separately from the
    status.
    """

    @pytest.mark.parametrize(
        "operation",
        VALIDATION_ERROR_OPERATIONS.values(),
        ids=list(VALIDATION_ERROR_OPERATIONS),
    )
    def test_operation_documents_a_422(self, operation):
        path, method = operation
        responses = rest_server.openapi()["paths"][path][method]["responses"]

        assert "422" in responses

    @pytest.mark.parametrize(
        "operation",
        VALIDATION_ERROR_OPERATIONS.values(),
        ids=list(VALIDATION_ERROR_OPERATIONS),
    )
    def test_the_documented_422_carries_the_validation_error_body(self, operation):
        path, method = operation
        response = rest_server.openapi()["paths"][path][method]["responses"]["422"]

        assert response["content"]["application/json"]["schema"] == {
            "$ref": "#/components/schemas/HTTPValidationError"
        }

    def test_the_validation_error_schema_is_published(self):
        # The ``$ref`` above is only useful if the target is in the spec.
        assert "HTTPValidationError" in rest_server.openapi()["components"]["schemas"]
