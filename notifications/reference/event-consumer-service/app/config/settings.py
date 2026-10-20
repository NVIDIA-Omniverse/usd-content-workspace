# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import os
from functools import cache
from typing import List, Self

from dotenv import load_dotenv
from notification_common.config import common_settings
from notification_common.errors.errors import ServerError
from pydantic import Field, model_validator
from pydantic_settings import SettingsConfigDict
from pydantic_settings.sources import (
    PydanticBaseSettingsSource,
    YamlConfigSettingsSource,
)

from app.config.event_permission import EventPermission

ENV_PREFIX = common_settings.CommonSettings.model_config["env_prefix"]

load_dotenv()


class Settings(common_settings.CommonSettings):
    """
    Settings for the App.
    NOTE: all environment variables will be prefixed
          with "OMNI_EVENTS_"
    """

    grpc_port: str = "[::]:50052"
    rest_api_enabled: bool = False
    rest_port: int = 8000
    # Readiness reserve, in STREAM slots on the NON-DURABLE pool: the
    # capacity surfaces subtract it from that pool's stream capacity
    # (max_client_capacity), not from a channel count. It equals a
    # channel count only at channel_client_limit=1. The default is 80,
    # sized against the 800-stream soft packing threshold this release
    # ships (100 pool_size x 8 client_limit): a 10% reserve leaving 720
    # usable slots for the scaling signal. The field name and its
    # OMNI_EVENTS_ env var are retained deliberately even though the unit
    # is streams, not channels: pydantic-settings silently ignores an
    # unknown OMNI_EVENTS_ var, so a rename would silently revert every
    # tuned deployment to the default. Two capacity-surface consumers read
    # it: the gRPC readiness servicer and the /metrics/channel-pool
    # autoscaler feed - one reserve, one unit. The reserve shapes the
    # SCALING signal, not admission: slots inside the reserve still serve
    # (pinned traffic up to the memory cap, re-routable up to the shed
    # threshold) - it only marks the pod unready to steer scale-out.
    # Must be >= 0: a zero reserve is meaningful ("unready only at real
    # exhaustion"), while a negative one would keep a pod ready past
    # exhaustion.
    readiness_min_available_channels: int = Field(
        default=80,
        ge=0,
        description=(
            "Readiness reserve in non-durable stream slots (not channels); "
            "subtracted from the non-durable pool's stream capacity. Equals "
            "a channel count only at channel_client_limit=1."
        ),
    )
    storage_permissions_endpoints: list[str] = []
    storage_permissions_enabled: bool = False
    event_permissions: List[EventPermission] = []
    non_durable_queue_ttl_seconds: int = 20
    # How often an idle consume/browse loop checks that its own AMQP
    # consumer is still attached. aio-pika delivers no notification when
    # the broker cancels a consumer or kills its channel, so this poll is
    # the only thing standing between a dead consumer and a stream that
    # stays open looking healthy forever. It bounds detection latency, so
    # it is the knob to lower for faster failover and raise to trim the
    # (already tiny) idle wakeup cost -- the check itself is in-process
    # and issues no broker traffic. Must be >= 1.0: sub-second polling
    # would spin the loop for no gain, since the conditions it detects
    # are broker-timescale events.
    consumer_liveness_poll_seconds: float = Field(default=15.0, ge=1.0)
    # Per-channel prefetch for durable consumers (both the with-acks and
    # server-delayed-ack paths). Bounds the in-flight messages nacked/
    # requeued per disconnect, keeping quorum x-delivery-count churn well
    # clear of the delivery-limit. Materially below the pool default of
    # 500; the tuning phase finalizes the production value. Must be >= 1:
    # a prefetch of 0 means "unlimited" to set_qos, defeating the bound.
    durable_consumer_prefetch_count: int = Field(default=20, ge=1)
    server_ack_delay_seconds: float = 2.0
    previously_visible_paths_ttl_seconds: int = 600
    unauthorized_recheck_seconds: int = 30
    redis_enabled: bool = False
    # max Ops queued in a single CacheWriter's buffer before
    # drop-oldest kicks in (units: ops). Must be >= 1: 0 makes
    # asyncio.Queue unbounded and silently disables backpressure.
    cache_writer_buffer_cap: int = Field(default=10000, ge=1)
    # Dead-letter capture topology (app-declared; SDD SR-1.5). The DLX is
    # set as x-dead-letter-exchange on durable origin queues and declared
    # at startup; the single global DLQ is declared and bound at startup.
    dead_letter_exchange_name: str = "events.dlx"
    dead_letter_queue_name: str = "events.dlq"
    # Single source of truth for the DLX->DLQ binding key AND the origin
    # queues' x-dead-letter-routing-key; both Subtasks read this one field.
    dead_letter_routing_key: str = "dead-letter"
    # DLQ's OWN self-bound limits. Defaults are deliberately generous so
    # dead-letters accumulate safely during the Epic-1-ships-before-Epic-2-
    # management-API gap (accepted interim state). >= 1 forbids a zero TTL/
    # max-length that would silently discard or never expire dead-letters.
    # CREATE-TIME ONLY: these become RabbitMQ queue arguments, which are
    # immutable after declaration, so changing either on a deployment whose
    # DLQ already exists does NOT retune the live queue -- the declare drifts
    # and the service logs the mismatch and carries on against the existing
    # queue. See the SDD's DLQ argument-drift section and the README's retune
    # guidance for how to actually change a bound.
    dead_letter_queue_message_ttl_ms: int = Field(default=1209600000, ge=1)
    dead_letter_queue_max_length: int = Field(default=1000000, ge=1)
    # Per-session prefetch for the DLQ browse consumer (SR-3.5). Caps how
    # many dead-letters a single browse session holds in-flight on the
    # single global DLQ; concurrent operators / replicas are competing
    # consumers, so this is the per-session page bound, distinct from
    # durable_consumer_prefetch_count. Must be >= 1: a prefetch of 0 means
    # "unlimited" to set_qos, defeating the bound.
    dead_letter_browse_prefetch_count: int = Field(default=10, ge=1)
    # Memory-safe non-durable stream cap (SDD SR-3.5 as amended). These
    # three fields feed app/lib/rabbitmq/stream_memory_cap.py, which turns
    # a fleet-wide broker memory budget into a per-pod admission cap. That
    # cap is the SOLE hard admission threshold on the non-durable path:
    # under two-tier admission the pool's own stream capacity
    # (pool_size x channel_client_limit) is only the soft packing and
    # scaling threshold - it is reported and it drives readiness and the
    # autoscaler feed, but it refuses no one, and streams above it are
    # admitted into the pool's overflow band. So the cap binding is the
    # normal ceiling, not a sign of misconfiguration. The pool bound
    # survives as a refusal only when the cap is incomputable (prefetch
    # <= 0), where there is no memory-safe number to enforce. When the cap
    # sits lower than the load the fleet needs to carry, the remedies are
    # unchanged: raise broker RAM (and this budget with it), or lower the
    # prefetch. The budget is stated in MiB and covers the whole fleet, so
    # the per-pod cap divides it by scaler_max_replicas.
    broker_stream_memory_budget_mib: int = Field(
        default=1024,
        ge=1,
        description=(
            "Broker memory budget in mebibytes (MiB) for worst-case "
            "non-durable prefetch buffers. Fleet-wide, not per pod: the "
            "per-pod stream cap divides this budget by scaler_max_replicas."
        ),
    )
    # The apportioning divisor for the fleet-wide budget above. Shares the
    # OMNI_EVENTS_SCALER_MAX_REPLICAS env var with the autoscaler so one
    # helm value (channelPoolAutoscaling.maxReplicas) feeds both consumers
    # and an operator never sets the same number twice; the default mirrors
    # ScalerSettings.scaler_max_replicas in notification_common, which is
    # the source of truth for that name and value. Helm templates the env
    # var into both the autoscaler Deployment and the service settings
    # configmap, sourcing channelPoolAutoscaling.maxReplicas when
    # autoscaling is enabled and replicaCount when it is not (with
    # autoscaling off, replicaCount IS the fleet ceiling). The default
    # below therefore applies only to non-helm deployments. Must be >= 1:
    # it is a divisor.
    scaler_max_replicas: int = Field(
        default=10,
        ge=1,
        description=(
            "The maximum number of service pods the fleet may run, in "
            "pods. Used as the divisor that turns the fleet-wide "
            "broker_stream_memory_budget_mib into a per-pod stream cap. "
            "Kept in step with the autoscaler by sharing the "
            "OMNI_EVENTS_SCALER_MAX_REPLICAS env var (helm templates one "
            "value into both)."
        ),
    )
    # Absolute per-pod override for operators who have measured their own
    # deployment; leaving it unset (None) means "compute the default from
    # the budget and the configured prefetch". None, not 0, is the
    # sentinel: 0 stays invalid because a real cap of 0 would reject every
    # stream. The name says _per_pod because the cap is enforced per
    # process while the budget it derives from is cluster-wide - the two
    # must not be confusable on a configmap line.
    max_non_durable_streams_per_pod: int | None = Field(
        default=None,
        ge=1,
        description=(
            "Absolute cap, in concurrent non-durable streams per pod, "
            "overriding the value computed from "
            "broker_stream_memory_budget_mib. Unset means compute it."
        ),
    )
    # Shed threshold for RE-ROUTABLE non-durable streams (SDD SR-3.7),
    # in concurrent non-durable streams per pod - the same unit as the
    # cap above, so the two knobs read as one scale. Above it, streams a
    # client can retry onto another pod (proxied gRPC and all REST/SSE)
    # are refused, reserving the band up to the cap for pinned streams
    # that scale-out cannot rescue. It is clamped into
    # [soft threshold, hard bound] at admission rather than validated
    # here, because both band endpoints are runtime values -
    # max_client_capacity comes from live pool stats and
    # memory_safe_stream_cap() from the prefetch, budget, and max-replica
    # settings - and neither is knowable at settings-validation time; an
    # out-of-band value is therefore accepted, not rejected. Setting it
    # at or above the cap disables the shed tier outright, which is the
    # documented posture for single-replica deployments or uniformly hot
    # fleets where there is nowhere to re-route to. Never applies to
    # durable admission, which has its own queue-backed lifecycle.
    max_re_routable_streams_per_pod: int | None = Field(
        default=None,
        ge=1,
        description=(
            "Per-pod stream count at which RE-ROUTABLE non-durable streams "
            "(proxied gRPC and all REST/SSE) are shed, reserving the band "
            "above it for pinned streams. Clamped at admission into "
            "[soft threshold, hard bound]. Unset means the midpoint of that "
            "band, computed by floor division as soft + (hard - soft) // 2: "
            "on a narrow band the re-routable half is the smaller one, and on "
            "a width-1 band the midpoint equals the soft threshold, so "
            "re-routable traffic sheds at the packing product."
        ),
    )

    model_config = SettingsConfigDict(
        env_prefix=ENV_PREFIX,
        yaml_file_encoding="utf-8",
        yaml_file=os.environ.get(f"{ENV_PREFIX}PERMISSIONS_YAML_FILE_PATH"),
    )

    @property
    def buffered_ttl_seconds(self) -> int:
        """
        Buffered TTL to avoid race conditions with the queue ttl
        """
        return 2 + self.non_durable_queue_ttl_seconds

    @model_validator(mode="after")
    def validate_auth_settings(self):
        """
        Validate authorization configuration is consistent.

        Each permission backend is an independent gate validated against
        only its own endpoint; enabling one gate does not require the
        other backend's endpoint, and a disabled gate's endpoint(s) are
        inert.
        """
        if self.permissions_enabled and not self.permissions_endpoint:
            raise ServerError(
                "permissions_enabled is true but permissions_endpoint is "
                "not configured. Set permissions_endpoint, or disable the "
                "permission service gate (permissions_enabled=false)."
            )

        if self.storage_permissions_enabled and not self.storage_permissions_endpoints:
            raise ServerError(
                "storage_permissions_enabled is true but "
                "storage_permissions_endpoints is not configured. Set "
                "storage_permissions_endpoints, or disable the storage "
                "permission gate (storage_permissions_enabled=false)."
            )

        return self

    @model_validator(mode="after")
    def validate_redis_settings(self):
        """
        Validate that redis_uri is provided when redis is enabled.
        """
        if self.redis_enabled and not self.redis_uri:
            raise ServerError("redis_uri must be provided when redis_enabled is True")
        return self

    @classmethod
    def settings_customise_sources(
        cls,
        settings_cls: Self,
        init_settings: PydanticBaseSettingsSource,
        env_settings: PydanticBaseSettingsSource,
        dotenv_settings: PydanticBaseSettingsSource,
        file_secret_settings: PydanticBaseSettingsSource,
    ) -> tuple[PydanticBaseSettingsSource, ...]:
        return YamlConfigSettingsSource(settings_cls), env_settings


settings = Settings()


@cache
def event_permissions_config(event_type: str) -> EventPermission | None:
    for permission in settings.event_permissions:
        if permission.event_type == event_type:
            return permission

    return None


@cache
def storage_endpoint():
    """
    For now, we only really allow a single storage provider for a deployment.

    In the future,
    routing rules defined by the storage APIS will be used to determine
    which endpoint to use for a specific event, based on its resource_id.
    """
    url = settings.storage_permissions_endpoints[0]

    if not url.endswith("/"):
        url += "/"

    return url
