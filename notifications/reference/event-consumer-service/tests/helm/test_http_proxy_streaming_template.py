# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

# Regression pins for the per-route policy surface of
# templates/http-proxy.yaml -- the streaming routes' load-balancer
# steering, retry policy, timeout envelope, backend protocol, and the
# X-Via-Proxy admission stamp, plus the correspondence between the
# proto's streaming RPCs and the routes that must exist for them (Issue
# b.33o).
#
# The `/` catch-all's OWN retry policy is pinned here too (Issue b.7mz):
# its connect-failure / refused-stream conditions and the narrow
# provably-unsent argument they rest on, the httpProxy.unary.retryPolicy.*
# tunables that size it, and the independence of its off-switch from the
# streaming one -- the two policies are separate helpers in _helpers.tpl
# and neither may drift onto the other's routes or be lost with it.
#
# Renders are scoped with --show-only templates/http-proxy.yaml because the
# whole-chart render also contains the Deployments, Services and ConfigMap,
# all owned by other test modules. --show-only is safe even at the default
# httpProxy.enabled=false: the template then emits a comment-only document
# rather than erroring, so the flag is set explicitly on every render here.
#
# tests/helm/test_http_proxy_template.py covers a different surface (the
# two resource NAMES and the rendered namespace, via whole-chart substring
# scans) and is deliberately left alone.

import shutil
import subprocess
from pathlib import Path

import pytest
import yaml
from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2

from tests.helm.conftest import disable_policy_hook, select_tokenless_auth

CHART_PATH = Path(__file__).resolve().parents[2] / "helm" / "event-consumer-service"

HTTP_PROXY_TEMPLATE = "templates/http-proxy.yaml"

# The chart renders one HTTPProxy per API surface, named
# <fullname>-<type>; `helm template ec` fixes the release name to "ec".
GRPC_PROXY = "ec-event-consumer-service-grpc"
REST_PROXY = "ec-event-consumer-service-rest"

# Route prefixes, as they appear under each route's conditions[].prefix.
_GRPC_SERVICE = "/nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService"
CONSUME_DURABLE_PREFIX = f"{_GRPC_SERVICE}/ConsumeDurableEvents"
CONSUME_DURABLE_WITH_ACKS_PREFIX = f"{_GRPC_SERVICE}/ConsumeDurableEventsWithAcks"
CONSUME_NON_DURABLE_PREFIX = f"{_GRPC_SERVICE}/ConsumeNonDurableEvents"
MANAGE_DLQ_PREFIX = f"{_GRPC_SERVICE}/ManageDeadLetteredEvents"
REST_STREAM_V1BETA_PREFIX = "/api/v1beta/events/stream"
REST_STREAM_V1_PREFIX = "/api/v1/events/stream"
CATCH_ALL_PREFIX = "/"

# The unary queue-CRUD RPC paths. These are NOT route prefixes: they have
# no dedicated route by design and must land on the `/` catch-all, with
# its short timeouts and -- the part that matters -- the catch-all's OWN
# narrow retry policy rather than the streaming routes' one. See
# TestUnaryRouteRetryPolicy on why only a provably-unsent failure may be
# replayed for a CRUD call, and TestStreamingRouteRetryPolicy on why the
# streaming policy must never reach these paths.
CREATE_DURABLE_QUEUE_PATH = f"{_GRPC_SERVICE}/CreateDurableQueue"
DELETE_DURABLE_QUEUE_PATH = f"{_GRPC_SERVICE}/DeleteDurableQueue"
UPDATE_DURABLE_QUEUE_PATH = f"{_GRPC_SERVICE}/UpdateDurableQueue"

# The six long-lived streaming routes. These are the routes whose
# steering and streaming-retry behavior the sibling Subtasks pin; the
# catch-all routes below carry the short configurable timeouts instead,
# no steering strategy at all, and a separate, much narrower retry policy
# of their own -- they must not pick the streaming block's configuration
# up, in either direction.
STREAMING_ROUTES = [
    (GRPC_PROXY, CONSUME_DURABLE_PREFIX),
    (GRPC_PROXY, CONSUME_DURABLE_WITH_ACKS_PREFIX),
    (GRPC_PROXY, CONSUME_NON_DURABLE_PREFIX),
    (GRPC_PROXY, MANAGE_DLQ_PREFIX),
    (REST_PROXY, REST_STREAM_V1BETA_PREFIX),
    (REST_PROXY, REST_STREAM_V1_PREFIX),
]

NON_STREAMING_ROUTES = [
    (GRPC_PROXY, CATCH_ALL_PREFIX),
    (REST_PROXY, CATCH_ALL_PREFIX),
]

ALL_ROUTES = STREAMING_ROUTES + NON_STREAMING_ROUTES

# Every route of the REST proxy, streaming and catch-all alike. The
# backend protocol is a whole-proxy property rather than a
# streaming-route one -- see TestBackendProtocol -- so the REST side is
# pinned across all of its routes at once.
REST_ROUTES = [route for route in ALL_ROUTES if route[0] == REST_PROXY]

# The routes each proxy renders, in render order. Pinned so an edit that
# adds, drops or reorders a route has to come through these tests.
ROUTE_ORDER = {
    GRPC_PROXY: [
        CONSUME_DURABLE_PREFIX,
        CONSUME_DURABLE_WITH_ACKS_PREFIX,
        CONSUME_NON_DURABLE_PREFIX,
        MANAGE_DLQ_PREFIX,
        CATCH_ALL_PREFIX,
    ],
    REST_PROXY: [
        REST_STREAM_V1BETA_PREFIX,
        REST_STREAM_V1_PREFIX,
        CATCH_ALL_PREFIX,
    ],
}

# The timeout envelope. These are hardcoded in the template, NOT
# operator-tunable: httpProxy.timeoutPolicy.* configures the catch-all
# routes only. The gRPC streaming routes idle at 240m rather than the SSE
# routes' 40s because gRPC keepalive PING frames do not reset Envoy's
# per-stream idle timer, so a quiet-but-healthy stream would be cut at
# 40s. The catch-all's 10s/10s is the values.yaml default and is what a
# streaming RPC WITHOUT a dedicated route silently falls back to -- the
# defect this module's recurrence guard exists to prevent.
STREAM_RESPONSE_TIMEOUT = "240m"
GRPC_STREAM_IDLE_TIMEOUT = "240m"
REST_STREAM_IDLE_TIMEOUT = "40s"
CATCH_ALL_RESPONSE_TIMEOUT = "10s"
CATCH_ALL_IDLE_TIMEOUT = "10s"

RESPONSE_TIMEOUT_KEY = "response"
IDLE_TIMEOUT_KEY = "idle"

# Contour's per-service upstream protocol, set on each entry of a route's
# services[]. Every gRPC route must carry it: without it Contour speaks
# HTTP/1.1 to the backend and gRPC does not work at all over the route.
# Every REST route must NOT: the REST backend is a uvicorn HTTP/1.1
# server, so h2c there would break the SSE routes just as surely.
PROTOCOL_KEY = "protocol"
H2C_PROTOCOL = "h2c"

GRPC_SERVICE_NAME = "EventConsumerService"


def _streaming_rpc_prefixes() -> set[str]:
    """The route prefix every streaming RPC of the service needs, from the proto.

    Read from the compiled descriptor rather than listed by hand, so a
    streaming RPC added to event_consumer.proto shows up here without any
    edit to this module -- which is what makes the recurrence guard below
    a guard rather than a second hand-maintained list that drifts the same
    way the chart did. `pkg.Service.Method` maps to the Contour prefix
    `/pkg.Service/Method`.
    """
    service = event_consumer_pb2.DESCRIPTOR.services_by_name[GRPC_SERVICE_NAME]
    return {
        f"/{service.full_name}/{method.name}"
        for method in service.methods
        if method.client_streaming or method.server_streaming
    }


def _unary_rpc_paths() -> set[str]:
    """The path every unary RPC of the service is served on, from the proto.

    The exact complement of _streaming_rpc_prefixes, read from the same
    descriptor for the same reason: a unary RPC added to
    event_consumer.proto has to show up here with no edit to this module.
    These are full RPC PATHS, not route prefixes -- the distinction is the
    whole point of the third-direction test below, which checks that no
    such path is prefix-captured by a dedicated streaming route. They
    belong on the catch-all.
    """
    service = event_consumer_pb2.DESCRIPTOR.services_by_name[GRPC_SERVICE_NAME]
    return {
        f"/{service.full_name}/{method.name}"
        for method in service.methods
        if not (method.client_streaming or method.server_streaming)
    }


# Load-balancer steering. Contour accepts Random, RoundRobin,
# WeightedLeastRequest, Cookie and RequestHash on a route's
# loadBalancerPolicy.strategy.
ROUND_ROBIN = "RoundRobin"
WEIGHTED_LEAST_REQUEST = "WeightedLeastRequest"

# Contour's documented fallback: "If an unknown strategy name is specified
# or no policy is supplied, the default RoundRobin policy is used."
CONTOUR_DEFAULT_STRATEGY = ROUND_ROBIN

STRATEGY_KEY = "httpProxy.streaming.loadBalancerPolicy.strategy"
GRPC_STRATEGY_KEY = "httpProxy.grpc.streaming.loadBalancerPolicy.strategy"

# The admission-class stamp. The value is a STRING: Envoy header values
# are strings, and an unquoted `true` in the template would parse as a
# bool here (and be rejected by Contour in a cluster).
PROXY_STAMP_HEADER = "X-Via-Proxy"
PROXY_STAMP_VALUE = "true"

# The one route the stamp belongs on, and every route it must stay off.
STAMPED_ROUTE = (GRPC_PROXY, CONSUME_NON_DURABLE_PREFIX)
UNSTAMPED_ROUTES = [route for route in ALL_ROUTES if route != STAMPED_ROUTE]

# Contour's retryPolicy CRD keys, named because the JSON keys DO NOT match
# the shorthand used in the SDD/Epic prose, and a wrong key renders happily
# and is then silently dropped by the CRD:
#   * `count` IS the retry count. The Go field is `NumRetries int64` but its
#     JSON tag is `count` (apis/projectcontour/v1/httpproxy.go). A chart
#     emitting a literal `numRetries:` key would be dropped and the count
#     would fall back to Envoy's default of 1, with no error anywhere. Do
#     NOT "fix" these tests to say numRetries.
#   * `retriableStatusCodes` is camelCase, and is respected ONLY when
#     `retryOn` contains the `retriable-status-codes` token -- either half
#     without the other is inert, which is why they are asserted separately.
#   * `perTryTimeout` is deliberately never emitted; see the pin below.
RETRY_POLICY_KEY = "retryPolicy"
RETRY_COUNT_KEY = "count"
RETRY_ON_KEY = "retryOn"
RETRIABLE_STATUS_CODES_KEY = "retriableStatusCodes"
PER_TRY_TIMEOUT_KEY = "perTryTimeout"

# The STREAMING retry CONDITIONS are fixed by the chart, not
# operator-tunable: they mirror this service's capacity-refusal contract
# exactly -- RESOURCE_EXHAUSTED on gRPC, 503 on REST.
GRPC_RETRY_CONDITION = "resource-exhausted"
REST_RETRY_CONDITION = "retriable-status-codes"
SHED_STATUS_CODE = 503

# Every condition the streaming policy can put in a retryOn, in either
# protocol. Used as the negative pin on the catch-all: the two policies
# are separate blocks in _helpers.tpl and neither list may drift into the
# other's routes.
STREAMING_RETRY_CONDITIONS = {GRPC_RETRY_CONDITION, REST_RETRY_CONDITION}

# Each streaming route paired with the condition ITS protocol's policy
# must carry. Used where a non-default render has to be shown to still
# carry the STREAMING policy rather than merely some retryPolicy: an
# emptied retryOn is read by Contour as 5xx, so key presence alone is not
# the claim being made.
STREAMING_ROUTE_CONDITIONS = [
    (
        proxy,
        prefix,
        GRPC_RETRY_CONDITION if proxy == GRPC_PROXY else REST_RETRY_CONDITION,
    )
    for proxy, prefix in STREAMING_ROUTES
]

# count: -1 does NOT disable retries. Contour maps it to numRetries = 0 and
# Envoy honours only NumRetries > 0, so -1 behaves exactly like 0 or unset:
# Envoy's default of 1 retry. Below Contour v1.22 the CRD schema rejects it
# outright. The real off-switch is retryPolicy.enabled: false, which omits
# the block entirely. RETRIES_DISABLED_COUNT keeps its historical name: it
# is the sentinel the chart must never emit, not a working off-switch. The
# chart's own default is 2 -- up to three placement attempts in total.
DEFAULT_RETRY_COUNT = 2
RETRIES_DISABLED_COUNT = -1
OVERRIDE_RETRY_COUNT = 5

RETRY_COUNT_VALUES_KEY = "httpProxy.streaming.retryPolicy.count"
RETRY_ENABLED_VALUES_KEY = "httpProxy.streaming.retryPolicy.enabled"
GRPC_RETRY_COUNT_VALUES_KEY = "httpProxy.grpc.streaming.retryPolicy.count"

# The UNARY catch-all route's retry policy (Issue b.7mz), rendered from
# the separate "event-consumer-service.unary-route-policies" helper.
#
# A LIST, not a set, and compared with ==: `retryOn` is an ordered YAML
# sequence and both members are load-bearing individually, so an
# accidental third token, a dropped one, or a substitution must all fail.
# The exactness is the safety property here -- see UNSAFE_RETRY_CONDITIONS
# below and TestUnaryRouteRetryPolicy's docstring.
#
# Same list on BOTH proxies: connect-failure is a TCP-level condition and
# refused-stream an HTTP/2 one that is simply inert against the HTTP/1.1
# REST backend, so the helper takes no `kind` branch.
UNARY_RETRY_CONDITIONS = ["connect-failure", "refused-stream"]

# Conditions that must NEVER appear on the catch-all. Each of them can
# retry a request the application already processed. The HTTP ones: 5xx
# and gateway-error cover app-generated 5xx responses,
# retriable-status-codes makes that operator-configurable,
# retriable-4xx adds Envoy's retriable 4xx class, retriable-headers lets
# the upstream RESPONSE ask for its own replay, and reset covers a stream
# reset that may arrive after the app acted. The gRPC ones --
# resource-exhausted (the streaming routes' capacity-refusal token),
# unavailable, deadline-exceeded, internal and cancelled -- all ride a
# trailer that Envoy sees only AFTER the request reached an application,
# so none of them distinguishes an undelivered call from a completed one.
# All eleven are accepted values in Contour's retryOn enum, so each is a
# render the chart could produce today.
#
# The catch-all fronts the non-idempotent durable-queue create / update /
# delete, and on a unary route Envoy cannot tell an already-processed
# refusal from an undelivered one -- so only provably-unsent conditions
# are admissible there.
#
# `unavailable` is the single most likely future widening: a terminating
# pod surfaces as gRPC UNAVAILABLE, so adding it reads like a more
# thorough fix for the very defect b.7mz is about. It is not -- gRPC
# UNAVAILABLE is not proof of non-delivery, since a server can return it
# from a call whose handler already ran, and the trailer looks identical
# either way. This list is the negative pin such a widening has to be
# argued past, in the Issue, rather than rebaselined here.
UNSAFE_RETRY_CONDITIONS = [
    "5xx",
    "retriable-status-codes",
    "reset",
    "gateway-error",
    "retriable-4xx",
    "retriable-headers",
    "resource-exhausted",
    "unavailable",
    "deadline-exceeded",
    "internal",
    "cancelled",
]

# The chart's unary default is 3 -- up to four delivery attempts in
# total, sized to survive several consecutive selections of terminating
# pods during a rollout. Deliberately a different number from the
# streaming DEFAULT_RETRY_COUNT of 2, so a cross-wired values key or a
# collapsed helper cannot pass by coincidence.
DEFAULT_UNARY_RETRY_COUNT = 3

UNARY_RETRY_COUNT_VALUES_KEY = "httpProxy.unary.retryPolicy.count"
UNARY_RETRY_ENABLED_VALUES_KEY = "httpProxy.unary.retryPolicy.enabled"
REST_UNARY_RETRY_COUNT_VALUES_KEY = "httpProxy.rest.unary.retryPolicy.count"
GRPC_UNARY_RETRY_ENABLED_VALUES_KEY = "httpProxy.grpc.unary.retryPolicy.enabled"

# The whole `unary` block and its `retryPolicy` child, blanked. Both are
# reachable states for an operator who overrides the block wholesale, and
# the helper's `| default dict` guards have to survive them: a render
# ERROR here would be a chart outage, not a missing retry.
UNARY_BLOCK_VALUES_KEY = "httpProxy.unary"
UNARY_RETRY_POLICY_VALUES_KEY = "httpProxy.unary.retryPolicy"

# The retry policy splits by protocol: the gRPC consume routes retry on
# RESOURCE_EXHAUSTED, the REST streaming routes on the 503 status code.
GRPC_STREAMING_ROUTES = [
    (proxy, prefix) for proxy, prefix in STREAMING_ROUTES if proxy == GRPC_PROXY
]
REST_STREAMING_ROUTES = [
    (proxy, prefix) for proxy, prefix in STREAMING_ROUTES if proxy == REST_PROXY
]

pytestmark = pytest.mark.skipif(
    shutil.which("helm") is None,
    reason="helm CLI not available",
)


def _render(*extra_set: str, show_only: str | None = None) -> str:
    args = ["helm", "template", "ec", str(CHART_PATH), "--namespace", "storage-api"]
    if show_only is not None:
        args.extend(["--show-only", show_only])
    # Before the caller's own overrides: helm takes the last --set, so an
    # explicit policyHook setting in extra_set still wins.
    disable_policy_hook(args)
    select_tokenless_auth(args)
    for kv in extra_set:
        args.extend(["--set", kv])
    result = subprocess.run(args, capture_output=True, text=True)
    assert result.returncode == 0, (
        f"helm template failed:\nstdout:\n{result.stdout}\n" f"stderr:\n{result.stderr}"
    )
    return result.stdout


def _routes(*extra_set: str) -> dict[str, dict[str, dict]]:
    """Index the rendered HTTPProxies as {proxy name: {route prefix: route}}.

    Parsed structurally rather than scanned as a substring because every
    question this module asks is about WHERE a policy landed, not whether
    the chart mentions it anywhere. A whole-chart substring scan cannot
    tell which of the six routes a `requestHeadersPolicy` attached to, so
    a stamp that leaked onto the REST catch-all would still match; it
    cannot see YAML types, so a `value: true` that parsed as a bool would
    read the same as the required string `"true"`; and it cannot tell
    `retryOn` apart from a future `retryOnPolicy` or `retryOnStatusCodes`.

    httpProxy.enabled=true is always injected -- at the default `false`
    the template renders a comment-only document with no routes at all.
    httpProxy.fqdn.domain defaults to `nodomain.none`, so the virtualhost
    needs no override.
    """
    out = _render("httpProxy.enabled=true", *extra_set, show_only=HTTP_PROXY_TEMPLATE)
    docs = [d for d in yaml.safe_load_all(out) if d]
    assert len(docs) == 2, f"expected exactly two documents, got {len(docs)}:\n{out}"
    for doc in docs:
        assert doc["kind"] == "HTTPProxy", f"unexpected kind:\n{out}"

    return {
        doc["metadata"]["name"]: {
            route["conditions"][0]["prefix"]: route for route in doc["spec"]["routes"]
        }
        for doc in docs
    }


@pytest.fixture(scope="module")
def default_routes() -> dict[str, dict[str, dict]]:
    """The routes of a stock render. Read-only; shared to avoid re-rendering."""
    return _routes()


class TestProxyStampRetention:
    """The X-Via-Proxy stamp on the gRPC non-durable route is permanent.

    Under SDD SR-3.7 this stamp is the admission CLASS MARKER, not a
    debugging convenience: a stamped gRPC stream arrived through Contour,
    so it is re-routable and is shed at the shed threshold, while an
    unstamped one is pinned to its pod and is refused only at the hard
    memory cap. The chart is the only thing that sets it -- Envoy's `set`
    semantics also overwrite it on every proxied request, so an external
    client cannot spoof its way into the re-routable class.

    This is NOT a rolling-upgrade transition artifact. Issue b.jfs
    ("Remove the dead chart-side X-Via-Proxy header injection..."), which
    proposed deleting the injection once _get_peer_id went away, is
    CLOSED as done/superseded precisely because SR-3.7 gave the stamp a
    permanent job. So this pin is permanent too: if a future cleanup
    deletes the `event-consumer-service.proxy-request-headers` template in
    _helpers.tpl or drops its include from http-proxy.yaml, these tests
    must fail, and the correct response is to RESTORE the injection --
    not to delete this test.

    The stamp must also stay on the gRPC ConsumeNonDurableEvents route
    ONLY. SR-3.7's recorded decision classifies REST/SSE streams as
    unconditionally re-routable in the service itself, so stamping a REST
    route would be redundant and would contradict the spec; that
    one-route-only scope is the cross-Task contract Task 2's `_is_proxied`
    is written against.
    """

    def test_non_durable_grpc_route_sets_a_request_headers_policy(self, default_routes):
        proxy, prefix = STAMPED_ROUTE

        assert "requestHeadersPolicy" in default_routes[proxy][prefix]

    def test_non_durable_grpc_route_stamps_exactly_the_proxy_header(
        self, default_routes
    ):
        proxy, prefix = STAMPED_ROUTE
        policy = default_routes[proxy][prefix]["requestHeadersPolicy"]

        assert policy["set"] == [
            {"name": PROXY_STAMP_HEADER, "value": PROXY_STAMP_VALUE}
        ]

    @pytest.mark.parametrize("proxy, prefix", UNSTAMPED_ROUTES)
    def test_no_other_route_sets_a_request_headers_policy(
        self, default_routes, proxy, prefix
    ):
        assert "requestHeadersPolicy" not in default_routes[proxy][prefix]

    def test_exactly_one_route_across_both_proxies_carries_the_stamp(
        self, default_routes
    ):
        # Set-valued companion to the per-route pin above. Adding a
        # streaming route is the moment the stamp is most likely to be
        # copy-pasted along with the rest of the route block --
        # ConsumeDurableEventsWithAcks and ManageDeadLetteredEvents both
        # borrow a DURABLE-pool channel, so stamping either would
        # misclassify a durable stream as re-routable non-durable traffic.
        stamped = {
            (proxy, prefix)
            for proxy, routes in default_routes.items()
            for prefix, route in routes.items()
            if "requestHeadersPolicy" in route
        }

        assert stamped == {STAMPED_ROUTE}


def _effective_strategy(route: dict) -> str:
    """The strategy Envoy will actually use for this route.

    Shape-agnostic on purpose. The chart currently renders the default
    EXPLICITLY (`strategy: RoundRobin`), but omitting the block entirely is
    byte-equivalent in effect, because Contour falls back to RoundRobin
    when no policy is supplied. A test written against the effective value
    keeps holding if the chart later switches between those two shapes,
    which is what makes it safe to pin "never WeightedLeastRequest by
    default" separately from the literal render.
    """
    policy = route.get("loadBalancerPolicy", {})
    return policy.get("strategy", CONTOUR_DEFAULT_STRATEGY)


@pytest.fixture(scope="module")
def wlr_routes() -> dict[str, dict[str, dict]]:
    """Routes with WeightedLeastRequest set fleet-wide. Read-only."""
    return _routes(f"{STRATEGY_KEY}={WEIGHTED_LEAST_REQUEST}")


@pytest.fixture(scope="module")
def empty_strategy_routes() -> dict[str, dict[str, dict]]:
    """Routes with the strategy explicitly blanked. Read-only."""
    return _routes(f"{STRATEGY_KEY}=")


@pytest.fixture(scope="module")
def grpc_only_wlr_routes() -> dict[str, dict[str, dict]]:
    """Routes with WeightedLeastRequest set on the gRPC proxy only."""
    return _routes(f"{GRPC_STRATEGY_KEY}={WEIGHTED_LEAST_REQUEST}")


class TestSteeringStrategy:
    """The streaming routes' load-balancer strategy, SDD SR-3.6.

    RoundRobin is the default and must stay the default. This is not
    inertia: WeightedLeastRequest is actively wrong for the deployment
    profile we ship into, because Envoy counts only the streams IT
    routed. Cluster-internal clients reach the service directly, so their
    long-lived streams are invisible to Envoy -- and least-request then
    becomes anti-correlated with true load. The pod carrying the most
    invisible streams looks emptiest and turns into a magnet for new
    external streams (the invisible-load magnet), which is strictly worse
    than 1/N placement for a sticky-heavy fleet. WeightedLeastRequest is
    documented as the recommendation for predominantly PROXIED
    deployments only, and is reachable as a value -- it is not the
    default. Before flipping this default, read SR-3.6 and check the
    per-pod stream gauges for sticky skew.

    Every strategy assertion here is exact string equality against a
    named constant, never a substring or a truthiness check, and that is
    load-bearing. Contour's HTTPProxy API specifies that "if an unknown
    strategy name is specified or no policy is supplied, the default
    RoundRobin policy is used" -- so a typo'd strategy renders happily,
    passes admission, and silently degrades to RoundRobin with no error
    anywhere in the cluster. A test that only checks the key is present,
    or that the rendered text contains "LeastRequest", would pass against
    `WeightedLeastRequst` and the operator's chosen steering would never
    take effect. Only the exact literal catches it.

    Steering is also strictly a STREAMING-route concern: these are the
    long-lived connections whose placement matters. A strategy that
    leaked onto the `/` CRUD catch-all is a scope bug, not a harmless
    extra -- Contour hashes the strategy into the Envoy cluster name, so
    it would split the catch-all's connection pool and rename its stats.
    """

    @pytest.mark.parametrize("proxy, prefix", STREAMING_ROUTES)
    def test_streaming_route_defaults_to_round_robin(
        self, default_routes, proxy, prefix
    ):
        policy = default_routes[proxy][prefix]["loadBalancerPolicy"]

        assert policy["strategy"] == ROUND_ROBIN

    @pytest.mark.parametrize("proxy, prefix", STREAMING_ROUTES)
    def test_streaming_route_default_is_never_weighted_least_request(
        self, default_routes, proxy, prefix
    ):
        # Shape-agnostic companion to the literal test above: survives a
        # later switch between explicit RoundRobin and omission.
        effective = _effective_strategy(default_routes[proxy][prefix])

        assert effective != WEIGHTED_LEAST_REQUEST

    @pytest.mark.parametrize("proxy, prefix", STREAMING_ROUTES)
    def test_override_steers_every_streaming_route(self, wlr_routes, proxy, prefix):
        policy = wlr_routes[proxy][prefix]["loadBalancerPolicy"]

        assert policy["strategy"] == WEIGHTED_LEAST_REQUEST

    @pytest.mark.parametrize("proxy, prefix", ALL_ROUTES)
    def test_empty_strategy_omits_the_policy_entirely(
        self, empty_strategy_routes, proxy, prefix
    ):
        assert "loadBalancerPolicy" not in empty_strategy_routes[proxy][prefix]

    @pytest.mark.parametrize("prefix", ROUTE_ORDER[GRPC_PROXY][:-1])
    def test_per_protocol_override_steers_the_grpc_streaming_routes(
        self, grpc_only_wlr_routes, prefix
    ):
        policy = grpc_only_wlr_routes[GRPC_PROXY][prefix]["loadBalancerPolicy"]

        assert policy["strategy"] == WEIGHTED_LEAST_REQUEST

    @pytest.mark.parametrize("prefix", ROUTE_ORDER[REST_PROXY][:-1])
    def test_per_protocol_override_leaves_the_rest_routes_on_the_default(
        self, grpc_only_wlr_routes, prefix
    ):
        policy = grpc_only_wlr_routes[REST_PROXY][prefix]["loadBalancerPolicy"]

        assert policy["strategy"] == ROUND_ROBIN

    @pytest.mark.parametrize("proxy, prefix", NON_STREAMING_ROUTES)
    def test_catch_all_carries_no_policy_by_default(
        self, default_routes, proxy, prefix
    ):
        assert "loadBalancerPolicy" not in default_routes[proxy][prefix]

    @pytest.mark.parametrize("proxy, prefix", NON_STREAMING_ROUTES)
    def test_catch_all_carries_no_policy_under_an_override(
        self, wlr_routes, proxy, prefix
    ):
        assert "loadBalancerPolicy" not in wlr_routes[proxy][prefix]

    @pytest.mark.parametrize("proxy", list(ROUTE_ORDER))
    def test_route_set_and_order_are_unchanged(self, default_routes, proxy):
        # The "no collateral change" pin. A literal golden-file compare is
        # deliberately avoided -- it would fail on any unrelated chart
        # edit -- so the route inventory and each route's other policies
        # are pinned individually here and just below.
        assert list(default_routes[proxy]) == ROUTE_ORDER[proxy]

    @pytest.mark.parametrize("proxy, prefix", ALL_ROUTES)
    def test_every_route_keeps_its_other_policies(self, default_routes, proxy, prefix):
        route = default_routes[proxy][prefix]

        assert {"timeoutPolicy", "responseHeadersPolicy", "services"} <= set(route)

    @pytest.mark.parametrize("proxy, prefix", ALL_ROUTES)
    def test_every_route_keeps_a_backend_service(self, default_routes, proxy, prefix):
        assert default_routes[proxy][prefix]["services"]


def _retry_conditions(route: dict) -> list[str]:
    """The route's rendered `retryOn` list, or [] when it carries no policy.

    Since Issue b.7mz both proxies render TWO different retry policies --
    the streaming one and the catch-all's -- so the interesting question
    stopped being "does this route have a retryPolicy" and became "which
    one". The set-valued scope tests below key on the conditions through
    this helper rather than branching on key presence at each call site.
    """
    # `or []` rather than a two-arg get default: a route that renders
    # `retryOn:` with no items under it parses to None, not a missing key,
    # so the default would never fire and every caller's `in` / `&` would
    # raise TypeError instead of failing the assertion it was written to
    # make. An emptied retryOn is a real render the callers pin against.
    return route.get(RETRY_POLICY_KEY, {}).get(RETRY_ON_KEY) or []


@pytest.fixture(scope="module")
def raised_count_routes() -> dict[str, dict[str, dict]]:
    """Routes with the retry count raised fleet-wide. Read-only."""
    return _routes(f"{RETRY_COUNT_VALUES_KEY}={OVERRIDE_RETRY_COUNT}")


@pytest.fixture(scope="module")
def zero_count_routes() -> dict[str, dict[str, dict]]:
    """Routes with the retry count explicitly set to 0. Read-only."""
    return _routes(f"{RETRY_COUNT_VALUES_KEY}=0")


@pytest.fixture(scope="module")
def retries_disabled_routes() -> dict[str, dict[str, dict]]:
    """Routes rendered with the retry policy switched off. Read-only."""
    return _routes(f"{RETRY_ENABLED_VALUES_KEY}=false")


@pytest.fixture(scope="module")
def grpc_only_raised_count_routes() -> dict[str, dict[str, dict]]:
    """Routes with the retry count raised on the gRPC proxy only."""
    return _routes(f"{GRPC_RETRY_COUNT_VALUES_KEY}={OVERRIDE_RETRY_COUNT}")


class TestStreamingRouteRetryPolicy:
    """The streaming routes' retry policy, SDD SR-3.7.

    This policy is what makes Task 2's class-aware shed invisible to
    clients: a shed stream is re-placed on a less-loaded pod by Envoy
    instead of surfacing a capacity refusal. The service's API contract
    already tells clients to reconnect until they land on a pod with
    capacity (event_consumer.proto) -- the retry policy performs that
    documented remedy transparently, before the client ever sees an error.

    Scope, asserted exactly rather than merely spot-checked: THIS policy
    lands on ALL FOUR streaming gRPC routes (ConsumeDurableEvents,
    ConsumeDurableEventsWithAcks, ConsumeNonDurableEvents and
    ManageDeadLetteredEvents) and on both REST streaming routes, and on
    nothing else. Every streaming RPC is included because
    RESOURCE_EXHAUSTED is the pod-local refusal on ALL of these paths, and
    each one raises it before any response commits -- the durable-pool
    paths are not exempt just because their pool is smaller.

    The `/` catch-all does carry a retryPolicy of its own (Issue b.7mz,
    pinned by TestUnaryRouteRetryPolicy), but it is a DIFFERENT policy and
    keeping the two apart is a CORRECTNESS boundary, not tidiness. The
    catch-all fronts the queue CRUD calls, and silently re-issuing a
    create or delete that may already have been applied is a hazard the
    client cannot detect -- so the catch-all retries only on
    connect-failure and refused-stream, both of which mean Envoy never
    delivered the request to an application at all. This policy's
    conditions do not have that property: resource-exhausted and the
    503-via-retriable-status-codes pair are app-GENERATED refusals, safe
    to replay only because a streaming route's refusal is raised
    pre-admission, before any response commits. That reasoning is
    specific to the streaming routes and does not transfer, so the tests
    below pin that neither list ever reaches the other's routes.

    The `enabled` switches are independent too: turning this one off must
    not disturb the catch-all's block, and vice versa. See
    TestRetryOffSwitchesAreIndependent.

    Every assertion reads the PARSED structure, never the rendered text.
    Contour honours `retriableStatusCodes` only when `retryOn` contains
    `retriable-status-codes`, so a chart carrying one without the other is
    inert while still containing both strings somewhere; and the status
    codes are `[]uint32`, so a quoted "503" is a render that only fails at
    apply time in the cluster. A substring scan sees neither problem.
    """

    @pytest.mark.parametrize("proxy, prefix", GRPC_STREAMING_ROUTES)
    def test_grpc_consume_route_carries_a_retry_policy(
        self, default_routes, proxy, prefix
    ):
        assert RETRY_POLICY_KEY in default_routes[proxy][prefix]

    @pytest.mark.parametrize("proxy, prefix", GRPC_STREAMING_ROUTES)
    def test_grpc_consume_route_retries_on_resource_exhausted(
        self, default_routes, proxy, prefix
    ):
        policy = default_routes[proxy][prefix][RETRY_POLICY_KEY]

        assert GRPC_RETRY_CONDITION in policy[RETRY_ON_KEY]

    @pytest.mark.parametrize("proxy, prefix", GRPC_STREAMING_ROUTES)
    def test_grpc_consume_route_carries_no_retriable_status_codes(
        self, default_routes, proxy, prefix
    ):
        # HTTP status codes are meaningless on the gRPC routes -- a gRPC
        # capacity refusal rides an HTTP 200 with a trailer.
        policy = default_routes[proxy][prefix][RETRY_POLICY_KEY]

        assert RETRIABLE_STATUS_CODES_KEY not in policy

    def test_the_grpc_resource_exhausted_route_set_is_exactly_the_streaming_rpcs(
        self, default_routes
    ):
        # Keyed on the CONDITION, not on the mere presence of a
        # retryPolicy: since b.7mz the catch-all carries a retryPolicy
        # too, so "has a retryPolicy" no longer separates the two
        # classes of route and a presence-keyed set would be satisfied by
        # the streaming policy leaking onto the catch-all.
        with_resource_exhausted = {
            prefix
            for prefix, route in default_routes[GRPC_PROXY].items()
            if GRPC_RETRY_CONDITION in _retry_conditions(route)
        }

        # Spelled out rather than derived from GRPC_STREAMING_ROUTES: this
        # is the one place the retry SCOPE is stated independently, so
        # widening the streaming-route list alone cannot quietly widen it.
        assert with_resource_exhausted == {
            CONSUME_DURABLE_PREFIX,
            CONSUME_DURABLE_WITH_ACKS_PREFIX,
            CONSUME_NON_DURABLE_PREFIX,
            MANAGE_DLQ_PREFIX,
        }

    def test_the_rest_status_code_retry_route_set_is_exactly_the_sse_routes(
        self, default_routes
    ):
        # The REST-side companion, stated independently for the same
        # reason. The REST catch-all carries the b.7mz policy, so the
        # separator here is the retriable-status-codes token.
        with_status_code_retry = {
            prefix
            for prefix, route in default_routes[REST_PROXY].items()
            if REST_RETRY_CONDITION in _retry_conditions(route)
        }

        assert with_status_code_retry == {
            REST_STREAM_V1BETA_PREFIX,
            REST_STREAM_V1_PREFIX,
        }

    @pytest.mark.parametrize("proxy, prefix", STREAMING_ROUTES)
    def test_streaming_route_never_carries_the_connect_level_conditions(
        self, default_routes, proxy, prefix
    ):
        # The boundary pointed the other way. A streaming stream that was
        # refused a connection is already covered by this route's own
        # policy; importing the catch-all's list here would be the first
        # symptom of the two helpers being collapsed into one.
        policy = default_routes[proxy][prefix][RETRY_POLICY_KEY]

        assert not set(UNARY_RETRY_CONDITIONS) & set(policy[RETRY_ON_KEY])

    @pytest.mark.parametrize("proxy, prefix", REST_STREAMING_ROUTES)
    def test_rest_streaming_route_carries_a_retry_policy(
        self, default_routes, proxy, prefix
    ):
        assert RETRY_POLICY_KEY in default_routes[proxy][prefix]

    @pytest.mark.parametrize("proxy, prefix", REST_STREAMING_ROUTES)
    def test_rest_streaming_route_retries_on_the_status_code_token(
        self, default_routes, proxy, prefix
    ):
        # Deliberately separate from the status-code test below: this token
        # is the PRECONDITION for retriableStatusCodes being read at all, so
        # a chart with the codes but not the token must fail distinctly
        # from one with neither.
        policy = default_routes[proxy][prefix][RETRY_POLICY_KEY]

        assert REST_RETRY_CONDITION in policy[RETRY_ON_KEY]

    @pytest.mark.parametrize("proxy, prefix", REST_STREAMING_ROUTES)
    def test_rest_streaming_route_retries_on_the_shed_status_code(
        self, default_routes, proxy, prefix
    ):
        policy = default_routes[proxy][prefix][RETRY_POLICY_KEY]

        assert SHED_STATUS_CODE in policy[RETRIABLE_STATUS_CODES_KEY]

    @pytest.mark.parametrize("proxy, prefix", REST_STREAMING_ROUTES)
    def test_rest_streaming_status_codes_render_as_integers(
        self, default_routes, proxy, prefix
    ):
        # []uint32 in the CRD: a quoted "503" is rejected only at apply
        # time in the cluster, so the render is where it has to be caught.
        codes = default_routes[proxy][prefix][RETRY_POLICY_KEY][
            RETRIABLE_STATUS_CODES_KEY
        ]

        assert all(type(code) is int for code in codes)

    @pytest.mark.parametrize("proxy, prefix", STREAMING_ROUTES)
    def test_streaming_route_never_emits_a_per_try_timeout(
        self, default_routes, proxy, prefix
    ):
        # Permanent pin. Envoy applies perTryTimeout to the INITIAL attempt
        # as well as to the retries, so any value shorter than a stream's
        # lifetime would cut healthy streams -- and these routes run a 240m
        # response timeout. There is no safe value to set here; the correct
        # response to this test failing is to remove the key, not to raise
        # it.
        policy = default_routes[proxy][prefix][RETRY_POLICY_KEY]

        assert PER_TRY_TIMEOUT_KEY not in policy

    @pytest.mark.parametrize("proxy, prefix", STREAMING_ROUTES)
    def test_default_render_sets_a_retry_count(self, default_routes, proxy, prefix):
        # Boundedness, asserted on the DEFAULT render across the next four
        # tests. The count=0 and enabled=false override tests below are
        # deliberately exempt -- they pin the dig-not-default choice and
        # the omission path respectively.
        policy = default_routes[proxy][prefix][RETRY_POLICY_KEY]

        assert RETRY_COUNT_KEY in policy

    @pytest.mark.parametrize("proxy, prefix", STREAMING_ROUTES)
    def test_default_retry_count_is_an_integer(self, default_routes, proxy, prefix):
        policy = default_routes[proxy][prefix][RETRY_POLICY_KEY]

        assert type(policy[RETRY_COUNT_KEY]) is int

    @pytest.mark.parametrize("proxy, prefix", STREAMING_ROUTES)
    def test_default_retry_count_does_not_disable_retries(
        self, default_routes, proxy, prefix
    ):
        # -1 is not an off-switch but is still wrong to emit: Contour maps
        # it to numRetries = 0 and Envoy guards NumRetries > 0, so it
        # silently collapses to Envoy's default of 1 -- a policy that reads
        # as deliberately tuned while being the default. It is also
        # schema-rejected below Contour v1.22.
        policy = default_routes[proxy][prefix][RETRY_POLICY_KEY]

        assert policy[RETRY_COUNT_KEY] != RETRIES_DISABLED_COUNT

    @pytest.mark.parametrize("proxy, prefix", STREAMING_ROUTES)
    def test_default_retry_count_is_bounded_and_positive(
        self, default_routes, proxy, prefix
    ):
        # Bounded is the whole point: at fleet-wide saturation the retries
        # must exhaust into the documented 503 / RESOURCE_EXHAUSTED and let
        # the client back off, rather than storm the remaining pods.
        policy = default_routes[proxy][prefix][RETRY_POLICY_KEY]

        assert policy[RETRY_COUNT_KEY] >= 1

    @pytest.mark.parametrize("proxy, prefix", STREAMING_ROUTES)
    def test_default_retry_count_is_two(self, default_routes, proxy, prefix):
        policy = default_routes[proxy][prefix][RETRY_POLICY_KEY]

        assert policy[RETRY_COUNT_KEY] == DEFAULT_RETRY_COUNT

    @pytest.mark.parametrize("proxy, prefix", NON_STREAMING_ROUTES)
    def test_catch_all_carries_none_of_the_streaming_retry_conditions(
        self, default_routes, proxy, prefix
    ):
        # Was "the catch-all carries no retryPolicy at all", which b.7mz
        # deliberately made false. The boundary it was really guarding --
        # no app-generated refusal is ever replayed on a CRUD call -- is
        # unchanged and is what is asserted now.
        #
        # Read through _retry_conditions so the claim stands on its own:
        # this test is about the streaming policy not LEAKING here, and
        # must not also fail (with a confusing KeyError) when the
        # catch-all's own policy has gone missing. That defect belongs to
        # TestUnaryRouteRetryPolicy, which names it precisely.
        conditions = _retry_conditions(default_routes[proxy][prefix])

        assert not STREAMING_RETRY_CONDITIONS & set(conditions)

    @pytest.mark.parametrize("proxy, prefix", NON_STREAMING_ROUTES)
    def test_catch_all_carries_no_retriable_status_codes(
        self, default_routes, proxy, prefix
    ):
        # The other half of the REST streaming policy. Contour reads this
        # key only alongside the retriable-status-codes token, so on its
        # own it is inert -- but its presence would mean the streaming
        # block had been rendered onto this route. Defaulted for the same
        # reason as the test above: a leak pin, not a presence pin.
        policy = default_routes[proxy][prefix].get(RETRY_POLICY_KEY, {})

        assert RETRIABLE_STATUS_CODES_KEY not in policy

    @pytest.mark.parametrize("proxy, prefix", NON_STREAMING_ROUTES)
    def test_streaming_count_override_does_not_reach_the_catch_all(
        self, raised_count_routes, proxy, prefix
    ):
        # The values keys are separate blocks: raising the STREAMING count
        # must leave the catch-all on its own default. A collapsed helper,
        # or an $unaryPolicies include pointed at the streaming settings
        # path, lands here.
        policy = raised_count_routes[proxy][prefix][RETRY_POLICY_KEY]

        assert policy[RETRY_COUNT_KEY] == DEFAULT_UNARY_RETRY_COUNT

    @pytest.mark.parametrize("proxy, prefix", STREAMING_ROUTES)
    def test_count_tunable_follows_its_values_key(
        self, raised_count_routes, proxy, prefix
    ):
        policy = raised_count_routes[proxy][prefix][RETRY_POLICY_KEY]

        assert policy[RETRY_COUNT_KEY] == OVERRIDE_RETRY_COUNT

    @pytest.mark.parametrize("proxy, prefix", STREAMING_ROUTES)
    def test_zero_count_renders_literally(self, zero_count_routes, proxy, prefix):
        # Guards the `dig`-not-`default` choice in _helpers.tpl: `default`
        # treats 0 as empty and would silently substitute the chart default
        # of 2, so an operator asking for Envoy's single-attempt behavior
        # would quietly get three.
        policy = zero_count_routes[proxy][prefix][RETRY_POLICY_KEY]

        assert policy[RETRY_COUNT_KEY] == 0

    @pytest.mark.parametrize("proxy, prefix", GRPC_STREAMING_ROUTES)
    def test_per_protocol_override_raises_the_grpc_count(
        self, grpc_only_raised_count_routes, proxy, prefix
    ):
        policy = grpc_only_raised_count_routes[proxy][prefix][RETRY_POLICY_KEY]

        assert policy[RETRY_COUNT_KEY] == OVERRIDE_RETRY_COUNT

    @pytest.mark.parametrize("proxy, prefix", REST_STREAMING_ROUTES)
    def test_per_protocol_override_leaves_the_rest_count_on_the_default(
        self, grpc_only_raised_count_routes, proxy, prefix
    ):
        policy = grpc_only_raised_count_routes[proxy][prefix][RETRY_POLICY_KEY]

        assert policy[RETRY_COUNT_KEY] == DEFAULT_RETRY_COUNT


@pytest.fixture(scope="module")
def unary_raised_count_routes() -> dict[str, dict[str, dict]]:
    """Routes with the UNARY retry count raised fleet-wide. Read-only."""
    return _routes(f"{UNARY_RETRY_COUNT_VALUES_KEY}={OVERRIDE_RETRY_COUNT}")


@pytest.fixture(scope="module")
def rest_only_unary_raised_count_routes() -> dict[str, dict[str, dict]]:
    """Routes with the unary retry count raised on the REST proxy only."""
    return _routes(f"{REST_UNARY_RETRY_COUNT_VALUES_KEY}={OVERRIDE_RETRY_COUNT}")


@pytest.fixture(scope="module")
def unary_zero_count_routes() -> dict[str, dict[str, dict]]:
    """Routes with the unary retry count explicitly set to 0. Read-only."""
    return _routes(f"{UNARY_RETRY_COUNT_VALUES_KEY}=0")


@pytest.fixture(scope="module")
def grpc_only_unary_retries_disabled_routes() -> dict[str, dict[str, dict]]:
    """Routes with the unary retry policy switched off on gRPC only."""
    return _routes(f"{GRPC_UNARY_RETRY_ENABLED_VALUES_KEY}=false")


@pytest.fixture(scope="module")
def no_unary_block_routes() -> dict[str, dict[str, dict]]:
    """Routes rendered with the whole httpProxy.unary block blanked."""
    return _routes(f"{UNARY_BLOCK_VALUES_KEY}=null")


@pytest.fixture(scope="module")
def no_unary_retry_policy_routes() -> dict[str, dict[str, dict]]:
    """Routes rendered with httpProxy.unary.retryPolicy blanked."""
    return _routes(f"{UNARY_RETRY_POLICY_VALUES_KEY}=null")


class TestUnaryRouteRetryPolicy:
    """The `/` catch-all route's retry policy, Issue b.7mz.

    The defect: during a rolling upgrade the -envoy Service publishes
    not-ready addresses, so Contour keeps a terminating pod in the
    load-balancer set until the pod OBJECT is deleted -- which is after
    the container has exited. In the window between the app's servers
    stopping and the pod going away, Envoy still selects that pod and
    every unary call landing there surfaces to the client as a 503 UF. A
    longer shutdown settle delay cannot close it, because it postpones the
    app stop and the pod deletion by the same amount; the remedy is this
    retry.

    Why a retry is admissible on a route fronting NON-IDEMPOTENT queue
    CRUD, which the pre-b.7mz chart deliberately left retry-free: both
    conditions are provably-unsent. connect-failure is a failed or
    timed-out upstream TCP connect, and refused-stream is an upstream
    REFUSED_STREAM reset, which a gRPC server's graceful GOAWAY produces
    for any stream above its last-stream-id. In both cases no application
    ever saw the request, so a replay on a fresh pod selection cannot
    duplicate work. That is a narrower claim than "retries are safe here"
    and the tests below are written to hold the chart to exactly it --
    UNSAFE_RETRY_CONDITIONS is pinned as a negative because the failure
    mode is a widened list, not a missing one, and a widened list looks
    like a better fix right up until it double-creates a queue.

    Exactness matters in the other direction too: an EMPTY retryOn is not
    "no conditions" -- Contour substitutes 5xx, precisely the unsafe
    policy ruled out above. So the list is asserted with == against
    UNARY_RETRY_CONDITIONS rather than by membership, and the only
    supported off-switch is enabled: false, which omits the block.

    The residual is known and deliberately NOT papered over by these
    tests: the REST leg is HTTP/1.1 and has no refused-stream analogue,
    so a pooled keep-alive connection to a terminating pod that Envoy
    finds dead at dispatch (response flag UC) is not retried. No
    assertion here claims otherwise.
    """

    @pytest.mark.parametrize("proxy, prefix", NON_STREAMING_ROUTES)
    def test_catch_all_carries_a_retry_policy(self, default_routes, proxy, prefix):
        assert RETRY_POLICY_KEY in default_routes[proxy][prefix]

    @pytest.mark.parametrize("proxy, prefix", NON_STREAMING_ROUTES)
    def test_catch_all_retries_on_exactly_the_connect_level_conditions(
        self, default_routes, proxy, prefix
    ):
        # List equality, not membership: order and exact membership are
        # both pinned, so a third token added "while we're here" fails.
        policy = default_routes[proxy][prefix][RETRY_POLICY_KEY]

        assert policy[RETRY_ON_KEY] == UNARY_RETRY_CONDITIONS

    @pytest.mark.parametrize("proxy, prefix", NON_STREAMING_ROUTES)
    @pytest.mark.parametrize("condition", UNSAFE_RETRY_CONDITIONS)
    def test_catch_all_never_retries_on_an_unsafe_condition(
        self, default_routes, proxy, prefix, condition
    ):
        # Redundant with the equality pin above by construction, and kept
        # anyway: this one names WHICH token was admitted in its failure
        # output, and it is the test a future widening of the list has to
        # be argued past rather than merely rebaselined. Defaulted read,
        # so it fails ONLY on an admitted unsafe token -- a route with no
        # policy at all satisfies this claim, and the separate presence
        # pin above is what catches that.
        conditions = _retry_conditions(default_routes[proxy][prefix])

        assert condition not in conditions

    @pytest.mark.parametrize("proxy, prefix", NON_STREAMING_ROUTES)
    def test_catch_all_never_emits_a_per_try_timeout(
        self, default_routes, proxy, prefix
    ):
        # Same permanent pin as on the streaming routes, and for a reason
        # that survives the short 10s envelope here: Envoy applies
        # perTryTimeout to the INITIAL attempt too, so it is a second,
        # silently-shorter response deadline layered under
        # httpProxy.timeoutPolicy.response. The correct response to this
        # failing is to remove the key.
        policy = default_routes[proxy][prefix][RETRY_POLICY_KEY]

        assert PER_TRY_TIMEOUT_KEY not in policy

    @pytest.mark.parametrize("proxy, prefix", NON_STREAMING_ROUTES)
    def test_default_unary_retry_count_is_three(self, default_routes, proxy, prefix):
        policy = default_routes[proxy][prefix][RETRY_POLICY_KEY]

        assert policy[RETRY_COUNT_KEY] == DEFAULT_UNARY_RETRY_COUNT

    @pytest.mark.parametrize("proxy, prefix", NON_STREAMING_ROUTES)
    def test_default_unary_retry_count_is_an_integer(
        self, default_routes, proxy, prefix
    ):
        # `count` is int64 in the CRD; a quoted "3" is rejected only at
        # apply time in the cluster, so the render is where to catch it.
        policy = default_routes[proxy][prefix][RETRY_POLICY_KEY]

        assert type(policy[RETRY_COUNT_KEY]) is int

    @pytest.mark.parametrize("proxy, prefix", NON_STREAMING_ROUTES)
    def test_default_unary_retry_count_does_not_disable_retries(
        self, default_routes, proxy, prefix
    ):
        # -1 is not an off-switch: Contour maps it to numRetries = 0 and
        # Envoy guards NumRetries > 0, so it collapses to Envoy's default
        # of 1 while reading as deliberately tuned.
        policy = default_routes[proxy][prefix][RETRY_POLICY_KEY]

        assert policy[RETRY_COUNT_KEY] != RETRIES_DISABLED_COUNT

    @pytest.mark.parametrize("proxy, prefix", NON_STREAMING_ROUTES)
    def test_shared_count_override_reaches_both_catch_alls(
        self, unary_raised_count_routes, proxy, prefix
    ):
        # Proves the number follows httpProxy.unary.retryPolicy.count
        # rather than being a hardcoded literal that matches the default.
        policy = unary_raised_count_routes[proxy][prefix][RETRY_POLICY_KEY]

        assert policy[RETRY_COUNT_KEY] == OVERRIDE_RETRY_COUNT

    def test_per_protocol_override_raises_the_rest_catch_all_count(
        self, rest_only_unary_raised_count_routes
    ):
        # The block participates in the per-protocol deep-merge like every
        # other key under httpProxy.
        route = rest_only_unary_raised_count_routes[REST_PROXY][CATCH_ALL_PREFIX]

        assert route[RETRY_POLICY_KEY][RETRY_COUNT_KEY] == OVERRIDE_RETRY_COUNT

    def test_per_protocol_override_leaves_the_grpc_catch_all_count_alone(
        self, rest_only_unary_raised_count_routes
    ):
        # The half of the merge that a shared-mutable-dict bug breaks: an
        # override applied to $config.proxySettings in place would raise
        # BOTH proxies and only this assertion would notice.
        route = rest_only_unary_raised_count_routes[GRPC_PROXY][CATCH_ALL_PREFIX]

        assert route[RETRY_POLICY_KEY][RETRY_COUNT_KEY] == DEFAULT_UNARY_RETRY_COUNT

    @pytest.mark.parametrize("proxy, prefix", NON_STREAMING_ROUTES)
    def test_zero_count_renders_literally(self, unary_zero_count_routes, proxy, prefix):
        # Guards the `dig`-not-`default` choice in the unary helper:
        # `default` treats 0 as empty and would silently substitute the
        # chart default of 3.
        policy = unary_zero_count_routes[proxy][prefix][RETRY_POLICY_KEY]

        assert policy[RETRY_COUNT_KEY] == 0

    def test_per_protocol_disable_omits_the_grpc_catch_all_block(
        self, grpc_only_unary_retries_disabled_routes
    ):
        route = grpc_only_unary_retries_disabled_routes[GRPC_PROXY][CATCH_ALL_PREFIX]

        assert RETRY_POLICY_KEY not in route

    def test_per_protocol_disable_leaves_the_rest_catch_all_block(
        self, grpc_only_unary_retries_disabled_routes
    ):
        route = grpc_only_unary_retries_disabled_routes[REST_PROXY][CATCH_ALL_PREFIX]

        assert RETRY_POLICY_KEY in route

    @pytest.mark.parametrize("proxy, prefix", NON_STREAMING_ROUTES)
    def test_blanking_the_whole_unary_block_omits_the_policy(
        self, no_unary_block_routes, proxy, prefix
    ):
        # The helper's `.settings.unary | default dict` guard. An operator
        # can reach this state with `--set httpProxy.unary=null` or by
        # replacing the block wholesale in a values file; a render ERROR
        # here would take the whole chart down, so the render succeeding
        # is as much of the contract as the omission is.
        assert RETRY_POLICY_KEY not in no_unary_block_routes[proxy][prefix]

    @pytest.mark.parametrize("proxy, prefix", NON_STREAMING_ROUTES)
    def test_blanking_the_unary_retry_policy_omits_the_policy(
        self, no_unary_retry_policy_routes, proxy, prefix
    ):
        # The `($unary.retryPolicy).enabled` parenthesised-nil guard, one
        # level down from the test above.
        assert RETRY_POLICY_KEY not in no_unary_retry_policy_routes[proxy][prefix]

    @pytest.mark.parametrize("proxy, prefix", STREAMING_ROUTES)
    def test_streaming_routes_keep_their_policy_when_the_unary_block_is_blank(
        self, no_unary_block_routes, proxy, prefix
    ):
        # Scope pin on the omission above: blanking the UNARY block must
        # cost the catch-all its policy and nothing else. The two policies
        # come from separate helpers, and a nil guard written one level
        # too high -- on the shared route-policy include rather than on
        # `.settings.unary` -- would take the streaming policy with it and
        # be invisible to a test that only looked at the catch-all.
        assert RETRY_POLICY_KEY in no_unary_block_routes[proxy][prefix]

    @pytest.mark.parametrize("proxy, prefix, condition", STREAMING_ROUTE_CONDITIONS)
    def test_streaming_retry_conditions_survive_a_blank_unary_block(
        self, no_unary_block_routes, proxy, prefix, condition
    ):
        # Presence alone would be satisfied by a surviving block with an
        # EMPTY retryOn, which Contour reads as 5xx -- so the surviving
        # policy's content is pinned too, per protocol.
        conditions = _retry_conditions(no_unary_block_routes[proxy][prefix])

        assert condition in conditions

    @pytest.mark.parametrize("proxy, prefix", NON_STREAMING_ROUTES)
    def test_catch_all_route_survives_a_blank_unary_block(
        self, no_unary_block_routes, proxy, prefix
    ):
        # Non-vacuity companion: the omission test above would also pass
        # if blanking httpProxy.unary had wiped the catch-all route --
        # or the whole render -- rather than just its retry policy.
        assert no_unary_block_routes[proxy][prefix]["services"]

    @pytest.mark.parametrize("proxy, prefix", STREAMING_ROUTES)
    def test_streaming_routes_keep_their_policy_when_the_retry_policy_is_blank(
        self, no_unary_retry_policy_routes, proxy, prefix
    ):
        # The same scope pin one level down, on the
        # httpProxy.unary.retryPolicy=null render.
        assert RETRY_POLICY_KEY in no_unary_retry_policy_routes[proxy][prefix]

    @pytest.mark.parametrize("proxy, prefix, condition", STREAMING_ROUTE_CONDITIONS)
    def test_streaming_retry_conditions_survive_a_blank_unary_retry_policy(
        self, no_unary_retry_policy_routes, proxy, prefix, condition
    ):
        # Content companion to the presence pin above, matching the one the
        # unary-block=null render already has: a nil guard written one level
        # too low would leave the streaming block standing with an EMPTY
        # retryOn, which Contour reads as 5xx -- retrying the very
        # non-idempotent calls this policy exists to keep retry-free.
        conditions = _retry_conditions(no_unary_retry_policy_routes[proxy][prefix])

        assert condition in conditions

    @pytest.mark.parametrize("proxy, prefix", NON_STREAMING_ROUTES)
    def test_catch_all_route_survives_a_blank_unary_retry_policy(
        self, no_unary_retry_policy_routes, proxy, prefix
    ):
        # The non-vacuity companion the retryPolicy=null omission test
        # never had.
        assert no_unary_retry_policy_routes[proxy][prefix]["services"]


@pytest.fixture(scope="module")
def unary_retries_disabled_routes() -> dict[str, dict[str, dict]]:
    """Routes with only the UNARY retry policy switched off. Read-only."""
    return _routes(f"{UNARY_RETRY_ENABLED_VALUES_KEY}=false")


@pytest.fixture(scope="module")
def all_retries_disabled_routes() -> dict[str, dict[str, dict]]:
    """Routes with BOTH retry off-switches thrown. Read-only."""
    return _routes(
        f"{RETRY_ENABLED_VALUES_KEY}=false",
        f"{UNARY_RETRY_ENABLED_VALUES_KEY}=false",
    )


class TestRetryOffSwitchesAreIndependent:
    """`httpProxy.streaming` and `httpProxy.unary` have separate off-switches.

    Before Issue b.7mz there was one retry policy in the chart and one
    switch, and a single parametrized test over ALL_ROUTES covered it.
    There are now two policies whose safety arguments are unrelated -- the
    streaming one replays a pre-admission capacity refusal, the unary one
    replays a request that was never delivered -- so an operator turning
    one off must not silently lose the other. That independence is the
    concern this class owns; the CONTENT of each policy is pinned by
    TestStreamingRouteRetryPolicy and TestUnaryRouteRetryPolicy.

    Omission is the documented -- and only -- way to turn either off.
    count: -1 does not do it: Contour maps it to numRetries = 0, i.e.
    Envoy's default of 1 retry.
    """

    @pytest.mark.parametrize("proxy, prefix", STREAMING_ROUTES)
    def test_streaming_switch_omits_every_streaming_block(
        self, retries_disabled_routes, proxy, prefix
    ):
        assert RETRY_POLICY_KEY not in retries_disabled_routes[proxy][prefix]

    @pytest.mark.parametrize("proxy, prefix", NON_STREAMING_ROUTES)
    def test_streaming_switch_leaves_the_catch_all_block(
        self, retries_disabled_routes, proxy, prefix
    ):
        assert RETRY_POLICY_KEY in retries_disabled_routes[proxy][prefix]

    @pytest.mark.parametrize("proxy, prefix", NON_STREAMING_ROUTES)
    def test_streaming_switch_leaves_the_catch_all_conditions_intact(
        self, retries_disabled_routes, proxy, prefix
    ):
        # Presence alone would be satisfied by an EMPTY retryOn, which
        # Contour reads as 5xx -- so the surviving block's content is
        # pinned here too, not just its existence.
        policy = retries_disabled_routes[proxy][prefix][RETRY_POLICY_KEY]

        assert policy[RETRY_ON_KEY] == UNARY_RETRY_CONDITIONS

    @pytest.mark.parametrize("proxy, prefix", NON_STREAMING_ROUTES)
    def test_unary_switch_omits_the_catch_all_block(
        self, unary_retries_disabled_routes, proxy, prefix
    ):
        assert RETRY_POLICY_KEY not in unary_retries_disabled_routes[proxy][prefix]

    @pytest.mark.parametrize("proxy, prefix", STREAMING_ROUTES)
    def test_unary_switch_leaves_every_streaming_block(
        self, unary_retries_disabled_routes, proxy, prefix
    ):
        assert RETRY_POLICY_KEY in unary_retries_disabled_routes[proxy][prefix]

    @pytest.mark.parametrize("proxy, prefix", ALL_ROUTES)
    def test_both_switches_omit_every_retry_policy_block(
        self, all_retries_disabled_routes, proxy, prefix
    ):
        assert RETRY_POLICY_KEY not in all_retries_disabled_routes[proxy][prefix]


class TestStreamingRouteTimeoutEnvelope:
    """The timeout envelope each route runs under.

    This is the surface Issue b.33o was actually about. A streaming RPC
    with no dedicated route does not fail loudly -- it falls to the `/`
    catch-all and inherits `response: 10s` / `idle: 10s`, so the stream is
    severed ten seconds after it goes quiet, but only for clients that
    reach the service THROUGH the proxy. Cluster-internal clients dial the
    Service directly and never see it, which is why the defect survived in
    the chart while the RPC looked healthy everywhere it was exercised.

    The envelope is hardcoded in the template on purpose:
    httpProxy.timeoutPolicy.* tunes the catch-all only. So these values
    are pinned literally, and the catch-all's own 10s/10s is pinned
    alongside them -- an edit that "unified" the two by pointing the
    streaming routes at the configurable block would otherwise pass every
    other test in this module.

    The RESPONSE timeout is uniform: all six streaming routes run 240m,
    so it is asserted once across STREAMING_ROUTES. The IDLE timeout is
    where the protocols diverge, so it stays split -- gRPC streaming idles
    at 240m where SSE idles at 40s, and the asymmetry is deliberate: gRPC
    keepalive PING frames are connection-level and do not reset Envoy's
    per-stream idle timer, so a healthy-but-quiet gRPC stream would be cut
    at 40s. SSE traffic keeps its stream alive with comment frames, which
    do.
    """

    @pytest.mark.parametrize("proxy, prefix", STREAMING_ROUTES)
    def test_streaming_route_takes_the_long_response_timeout(
        self, default_routes, proxy, prefix
    ):
        policy = default_routes[proxy][prefix]["timeoutPolicy"]

        assert policy[RESPONSE_TIMEOUT_KEY] == STREAM_RESPONSE_TIMEOUT

    @pytest.mark.parametrize("proxy, prefix", GRPC_STREAMING_ROUTES)
    def test_grpc_streaming_route_takes_the_long_idle_timeout(
        self, default_routes, proxy, prefix
    ):
        policy = default_routes[proxy][prefix]["timeoutPolicy"]

        assert policy[IDLE_TIMEOUT_KEY] == GRPC_STREAM_IDLE_TIMEOUT

    @pytest.mark.parametrize("proxy, prefix", REST_STREAMING_ROUTES)
    def test_rest_streaming_route_keeps_the_sse_idle_timeout(
        self, default_routes, proxy, prefix
    ):
        policy = default_routes[proxy][prefix]["timeoutPolicy"]

        assert policy[IDLE_TIMEOUT_KEY] == REST_STREAM_IDLE_TIMEOUT

    @pytest.mark.parametrize("proxy, prefix", NON_STREAMING_ROUTES)
    def test_catch_all_keeps_the_short_response_timeout(
        self, default_routes, proxy, prefix
    ):
        policy = default_routes[proxy][prefix]["timeoutPolicy"]

        assert policy[RESPONSE_TIMEOUT_KEY] == CATCH_ALL_RESPONSE_TIMEOUT

    @pytest.mark.parametrize("proxy, prefix", NON_STREAMING_ROUTES)
    def test_catch_all_keeps_the_short_idle_timeout(
        self, default_routes, proxy, prefix
    ):
        policy = default_routes[proxy][prefix]["timeoutPolicy"]

        assert policy[IDLE_TIMEOUT_KEY] == CATCH_ALL_IDLE_TIMEOUT


class TestBackendProtocol:
    """The upstream protocol Contour speaks to the backend, per route.

    `services[].protocol: h2c` is what makes a route carry gRPC at all:
    without it Contour proxies HTTP/1.1 upstream and the RPC does not work
    over the route -- a gRPC route added without this line is worse than
    no route. Its absence on the REST proxy is equally load-bearing in the
    other direction: the REST backend is a uvicorn HTTP/1.1 server, so an
    h2c that leaked onto the SSE routes would break them the same way.

    The two proxies reach their value by two INDEPENDENT template
    mechanisms, which is why the catch-all is pinned separately here
    rather than riding along on the streaming routes. The four gRPC
    streaming routes each hardcode a literal `protocol: h2c` line. The `/`
    catch-all is emitted once for BOTH proxies from the shared route block
    at the end of http-proxy.yaml, and derives the key from a ternary on
    the per-proxy `$config.protocol` -- "h2c" for gRPC, "" for REST -- so
    an edit there moves the gRPC catch-all and the REST routes together
    while leaving all four literals untouched. Inverting that ternary
    rendered the gRPC catch-all with no protocol at all, silently breaking
    the three unary queue-CRUD RPCs through the proxy, and every test in
    this module still passed; these two catch-all-aware tests are why it
    cannot happen again.
    """

    @pytest.mark.parametrize("proxy, prefix", GRPC_STREAMING_ROUTES)
    def test_grpc_streaming_route_speaks_h2c_to_the_backend(
        self, default_routes, proxy, prefix
    ):
        services = default_routes[proxy][prefix]["services"]

        assert all(service[PROTOCOL_KEY] == H2C_PROTOCOL for service in services)

    def test_grpc_catch_all_speaks_h2c_to_the_backend(self, default_routes):
        # The catch-all fronts the three unary queue-CRUD RPCs, which are
        # gRPC exactly like the streaming ones -- it is not an HTTP/1.1
        # fallback route. Distinct from the test above because it exercises
        # the ternary rather than the literals.
        services = default_routes[GRPC_PROXY][CATCH_ALL_PREFIX]["services"]

        assert all(service[PROTOCOL_KEY] == H2C_PROTOCOL for service in services)

    @pytest.mark.parametrize("proxy, prefix", REST_ROUTES)
    def test_rest_route_carries_no_backend_protocol(
        self, default_routes, proxy, prefix
    ):
        # The REST catch-all shares the ternary with the gRPC one, so a
        # change that hardcodes h2c into the shared block -- or a REST
        # `protocol` value added to the $proxies dict -- lands here. The
        # SSE routes are included because they are the copy-paste target
        # for the gRPC routes' literal line.
        services = default_routes[proxy][prefix]["services"]

        assert all(PROTOCOL_KEY not in service for service in services)


class TestEveryStreamingRpcHasADedicatedRoute:
    """Recurrence guard for Issue b.33o, read from the proto descriptor.

    b.33o was not a typo, it was a MISSING entry: ConsumeDurableEventsWithAcks
    and ManageDeadLetteredEvents were added to event_consumer.proto and the
    chart was never widened, so both landed on the catch-all's 10s idle
    timeout through the proxy. Nothing in the chart, the proto, or the
    service could notice -- the two artifacts have no link between them.

    These tests are that link. The RPC side is derived from the compiled
    descriptor, never listed by hand, so the next streaming RPC added to
    the proto fails this test on the commit that adds it rather than in
    production months later. The correct response to a failure here is to
    add the route to templates/http-proxy.yaml (and to STREAMING_ROUTES
    and ROUTE_ORDER above), not to relax the assertion.

    The correspondence is pinned in THREE directions. A missing route is
    the b.33o defect itself. A leftover route with no matching RPC is the
    same defect one rename later: Contour matches conditions[].prefix as a
    plain STRING prefix, so a renamed RPC silently stops matching its old
    route and drops onto the catch-all, while the stale route keeps
    rendering and looking like coverage.

    The third direction is that same string-prefix matching pointed the
    other way: because Contour sorts routes longest-prefix-first and does
    not match on path SEGMENTS, a future unary RPC whose name merely
    extends a streaming one -- ConsumeDurableEventsStatus, say -- would be
    captured by the ConsumeDurableEvents route by name coincidence. It
    would inherit the 240m envelope, which is merely odd, and the
    STREAMING retryPolicy, which is not. Since Issue b.7mz the catch-all
    carries a retryPolicy of its own, so the hazard is no longer "a unary
    call would acquire retries" -- it is that it would acquire the WRONG
    ones. The catch-all's policy replays only failures Envoy raised
    before the request reached an application; the streaming policy
    replays an app-generated RESOURCE_EXHAUSTED, which on a queue-CRUD
    call may follow work that was really done and is undetectable to the
    client. So no unary RPC path may be prefix-captured by any dedicated
    route.
    """

    def test_the_descriptor_yields_the_known_streaming_rpcs(self):
        # Non-vacuity pin: the two set-difference tests below both pass
        # trivially if the descriptor traversal ever yields nothing.
        assert _streaming_rpc_prefixes() == {
            CONSUME_DURABLE_PREFIX,
            CONSUME_DURABLE_WITH_ACKS_PREFIX,
            CONSUME_NON_DURABLE_PREFIX,
            MANAGE_DLQ_PREFIX,
        }

    def test_every_streaming_rpc_has_a_dedicated_grpc_route(self, default_routes):
        missing = _streaming_rpc_prefixes() - set(default_routes[GRPC_PROXY])

        assert not missing, f"streaming RPCs left on the catch-all route: {missing}"

    def test_no_dedicated_grpc_route_outlives_its_rpc(self, default_routes):
        dedicated = set(default_routes[GRPC_PROXY]) - {CATCH_ALL_PREFIX}
        stale = dedicated - _streaming_rpc_prefixes()

        assert not stale, f"routes matching no streaming RPC: {stale}"

    def test_the_descriptor_yields_the_known_unary_rpcs(self):
        # Non-vacuity pin, the companion of the streaming one above: the
        # capture test below passes trivially if the traversal ever yields
        # no unary RPCs at all.
        assert _unary_rpc_paths() == {
            CREATE_DURABLE_QUEUE_PATH,
            DELETE_DURABLE_QUEUE_PATH,
            UPDATE_DURABLE_QUEUE_PATH,
        }

    def test_no_unary_rpc_is_captured_by_a_dedicated_route(self, default_routes):
        # str.startswith, not a segment comparison, because that is
        # exactly how Contour matches a `prefix:` condition. The correct
        # response to a failure here is to leave the unary RPC on the
        # catch-all -- either by naming it so it does not extend a
        # streaming RPC's name, or by giving it its own route with the
        # catch-all's timeouts and the catch-all's narrow connect-level
        # retryPolicy, never the streaming one.
        dedicated = set(default_routes[GRPC_PROXY]) - {CATCH_ALL_PREFIX}
        captured = {
            (path, prefix)
            for path in _unary_rpc_paths()
            for prefix in dedicated
            if path.startswith(prefix)
        }

        assert not captured, f"unary RPCs captured by a streaming route: {captured}"
