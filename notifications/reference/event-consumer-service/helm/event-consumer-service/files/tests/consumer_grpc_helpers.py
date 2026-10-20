# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Common test functions for Event Consumer Service gRPC smoke tests."""

import logging
import time

import grpc
from utils import generate_queue_id, timeout_grpc_call

logger = logging.getLogger(__name__)

# Wall-clock budget shared by the unary durable-queue CRUD calls below. A
# rolling upgrade can leave the proxy routing to a pod whose servers have
# already stopped; those calls come back as UNAVAILABLE, and every caller
# here tolerates having its request replayed.
_UNAVAILABLE_RETRY_SECONDS = 60
_UNAVAILABLE_RETRY_DELAY = 2


def retry_on_unavailable_grpc(
    grpc_func, *args, not_found_after_retry_ok=False, **kwargs
):
    """Call a unary gRPC method, retrying UNAVAILABLE, and return its result.

    UNAVAILABLE is the status a call routed to a pod whose servers have
    already stopped comes back as, and it is the only status retried. Any
    other RpcError -- INVALID_ARGUMENT or NOT_FOUND, say -- is raised at
    once rather than being retried for a minute and then reported as a
    timeout.

    Retrying is safe here because every caller tolerates a replay, not
    because the request is known to be undelivered. A terminating pod can
    accept an RPC, process it, and then tear the connection down mid-drain
    before the response is read, which the client sees as UNAVAILABLE
    exactly as it sees a call that never arrived; the two are not
    distinguishable from here. The callers tolerate that: create supplies
    its own queue_id and is therefore ensure-exists, and update resolves
    to RabbitMQ bind / unbind calls that are idempotent. Delete is the
    outlier -- a replayed delete whose first attempt actually succeeded
    comes back NOT_FOUND -- so it opts in to ``not_found_after_retry_ok``.
    Do not route a call that tolerates neither through this helper.

    ``not_found_after_retry_ok`` takes effect ONLY once at least one
    UNAVAILABLE retry has happened, and then treats NOT_FOUND as success
    by returning None. A NOT_FOUND on the first attempt is still raised,
    so a genuinely wrong queue_id stays a real failure.

    The budget is wall-clock (``_UNAVAILABLE_RETRY_SECONDS``); exhausting
    it raises an AssertionError naming the budget and the last failure.
    """
    deadline = time.monotonic() + _UNAVAILABLE_RETRY_SECONDS
    last_error = None
    retried = False
    while True:
        try:
            return grpc_func(*args, **kwargs)
        except grpc.RpcError as e:
            if (
                retried
                and not_found_after_retry_ok
                and e.code() == grpc.StatusCode.NOT_FOUND
            ):
                logger.info(
                    "Consumer gRPC call returned NOT_FOUND after an "
                    "UNAVAILABLE retry. Treating an earlier attempt as "
                    "having succeeded."
                )
                return None
            if e.code() != grpc.StatusCode.UNAVAILABLE:
                raise
            last_error = e
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise AssertionError(
                f"Consumer gRPC call still UNAVAILABLE after the "
                f"{_UNAVAILABLE_RETRY_SECONDS}s retry budget. "
                f"Last failure: {last_error}"
            ) from last_error
        logger.info(
            "Consumer gRPC call UNAVAILABLE. Retrying, %.0fs of the %ss "
            "budget left...",
            remaining,
            _UNAVAILABLE_RETRY_SECONDS,
        )
        time.sleep(min(_UNAVAILABLE_RETRY_DELAY, remaining))
        retried = True


def retry_on_resource_exhausted_grpc(
    grpc_func, *args, max_retries=30, retry_delay=2, **kwargs
):
    """Retry a gRPC consumer call if it fails with RESOURCE_EXHAUSTED."""
    last_error = None
    for attempt in range(max_retries + 1):
        try:
            return grpc_func(*args, **kwargs)
        except grpc.RpcError as e:
            if e.code() == grpc.StatusCode.RESOURCE_EXHAUSTED:
                last_error = e
                if attempt < max_retries:
                    logger.info(
                        "Consumer gRPC RESOURCE_EXHAUSTED (attempt %s/%s), "
                        "retry in %ss",
                        attempt + 1,
                        max_retries + 1,
                        retry_delay,
                    )
                    time.sleep(retry_delay)
                    continue
            raise
    if last_error:
        raise last_error


def add_filters(request, resource_base_url):
    """Add filter_groups for omni.storage.created to the request."""
    from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2

    lazy = event_consumer_pb2.ResourceFilter.FilterType.FILTER_TYPE_STARTS_WITH_LAZY
    fg = event_consumer_pb2.FilterGroup(event_type="omni.storage.created")
    fg.filters.append(
        event_consumer_pb2.ResourceFilter(
            filter_type=lazy,
            resource_id=resource_base_url,
        )
    )
    request.filter_groups.append(fg)


def create_durable_queue_grpc(stub, grpc_metadata, resource_base_url):
    """Create a durable queue via gRPC and return its queue_id.

    Retries an UNAVAILABLE within the shared retry budget, which is what a
    call routed to a pod that is shutting down during a rolling upgrade
    comes back as. The call supplies its own queue_id so the retry is
    ensure-exists rather than a second queue: a repeat with an id already
    in use returns that existing queue with created=false.
    """
    from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2

    queue_id = generate_queue_id()
    req = event_consumer_pb2.CreateDurableQueueRequest(queue_id=queue_id)
    add_filters(req, resource_base_url)
    response = retry_on_unavailable_grpc(
        timeout_grpc_call, stub.CreateDurableQueue, req, metadata=grpc_metadata
    )
    return response.queue_id


def delete_durable_queue_grpc(grpc_metadata, stub, queue_id):
    """Delete a durable queue via gRPC.

    Retries an UNAVAILABLE within the shared retry budget, as create does.
    Delete is the one caller a replay is not naturally idempotent for: if
    the first attempt reached the application and only its response was
    lost, the replay finds the queue already gone, so a NOT_FOUND after a
    retry is accepted as success.
    """
    from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2

    req = event_consumer_pb2.DeleteDurableQueueRequest(queue_id=queue_id)
    # NOT_FOUND only counts as success after a retry; on the first attempt
    # it still raises, so a wrong queue_id remains a real failure.
    retry_on_unavailable_grpc(
        timeout_grpc_call,
        stub.DeleteDurableQueue,
        req,
        metadata=grpc_metadata,
        not_found_after_retry_ok=True,
    )


def update_durable_queue_grpc(stub, grpc_metadata, queue_id, resource_base_url):
    """Update a durable queue's filters (lazy -> greedy) via gRPC.

    Retries an UNAVAILABLE within the shared retry budget, as create and
    delete do.
    """
    from nvidia.omniverse.notifications.consumer.v1beta import event_consumer_pb2

    lazy = event_consumer_pb2.ResourceFilter.FilterType.FILTER_TYPE_STARTS_WITH_LAZY
    greedy = event_consumer_pb2.ResourceFilter.FilterType.FILTER_TYPE_STARTS_WITH_GREEDY
    current = event_consumer_pb2.FilterGroup(event_type="omni.storage.created")
    current.filters.append(
        event_consumer_pb2.ResourceFilter(
            filter_type=lazy,
            resource_id=resource_base_url,
        )
    )
    new_fg = event_consumer_pb2.FilterGroup(event_type="omni.storage.created")
    new_fg.filters.append(
        event_consumer_pb2.ResourceFilter(
            filter_type=greedy,
            resource_id=resource_base_url,
        )
    )
    req = event_consumer_pb2.UpdateDurableQueueRequest(
        queue_id=queue_id,
        current_filter_groups=[current],
        new_filter_groups=[new_fg],
    )
    retry_on_unavailable_grpc(
        timeout_grpc_call, stub.UpdateDurableQueue, req, metadata=grpc_metadata
    )
