# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
The stand-in reconnect token every token-hygiene test asserts the absence
of, and the broker queue name that embeds it.

A non-durable queue's id IS the stream's reconnect token. The token is not
a credential: presenting it requires a valid bearer token and every
delivered event is authorized per principal by the permissions policy. It
is the queue's identifier, so an *authorized* principal that obtains it
could attach to the same stream and take a share of its events; it is kept
out of telemetry as defense in depth. Roughly a dozen test modules -- the
servicer, the REST handler, both consumers, the access log, the Redis
cache, the cache writer, the naming module -- each assert that this exact
string never reaches a log record, a span attribute, or a metric attribute.
Declaring it once is what keeps those assertions talking about the same
value: a module that spelled its own literal slightly differently would
still pass while pinning nothing the others pin.

The spelling is deliberately unmistakable, so a leak in a captured log line
reads as a leak rather than as plausible test data.
"""

from app.lib.queue_names import NON_DURABLE_SUFFIX, QUEUE_NAME_PREFIX

#: The queue identifier itself.
RECONNECT_TOKEN = "s3cr3t-reconnect-token-do-not-log"

#: The broker queue name the token lands in. Composed from the naming
#: module's own constants so a change to either half is a change here too,
#: rather than a literal that quietly stops matching the shape
#: ``redact_queue_name`` recognises.
NON_DURABLE_QUEUE_NAME = f"{QUEUE_NAME_PREFIX}:{RECONNECT_TOKEN}{NON_DURABLE_SUFFIX}"
