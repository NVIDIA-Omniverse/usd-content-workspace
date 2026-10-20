// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use fred::types::scripts::Script;

/// Atomically acquire a lease (SET NX PX).
/// KEYS\[1\] = lease key, ARGV\[1\] = token, ARGV\[2\] = ttl_ms
/// Returns 1 if acquired, 0 if already held.
const LEASE_ACQUIRE: &str = r#"
if redis.call('SET', KEYS[1], ARGV[1], 'NX', 'PX', ARGV[2]) then
  return 1
else
  return 0
end
"#;

/// Compare-and-swap write guarded by lease token.
/// KEYS\[1\] = data key, KEYS\[2\] = lease key
/// ARGV\[1\] = expected token, ARGV\[2\] = envelope json, ARGV\[3\] = ttl_ms
/// Returns 1 if written, 0 if lease was lost.
const LEASE_CAS_WRITE: &str = r#"
local current = redis.call('GET', KEYS[2])
if current ~= ARGV[1] then
  return 0
end
redis.call('SET', KEYS[1], ARGV[2], 'PX', ARGV[3])
redis.call('DEL', KEYS[2])
return 1
"#;

/// Release a lease only when it is still owned by the supplied token.
/// KEYS\[1\] = lease key, ARGV\[1\] = expected token
/// Returns 1 if released, 0 if the lease was absent or owned by another fill.
const LEASE_RELEASE: &str = r#"
local current = redis.call('GET', KEYS[1])
if current ~= ARGV[1] then
  return 0
end
redis.call('DEL', KEYS[1])
return 1
"#;

/// Insert guarded by tombstone existence.
/// KEYS\[1\] = data key, KEYS\[2\] = tombstone key
/// ARGV\[1\] = envelope json, ARGV\[2\] = ttl_ms
/// Returns 1 if written, 0 if tombstone blocked.
const TOMBSTONE_GUARDED_INSERT: &str = r#"
if redis.call('EXISTS', KEYS[2]) == 1 then
  return 0
end
redis.call('SET', KEYS[1], ARGV[1], 'PX', ARGV[2])
return 1
"#;

/// Atomically invalidate a key: delete data + lease, plant tombstone.
/// KEYS\[1\] = data key, KEYS\[2\] = lease key, KEYS\[3\] = tombstone key
/// ARGV\[1\] = tombstone ttl_ms
/// Returns 1 always.
const INVALIDATE_KEY: &str = r#"
redis.call('DEL', KEYS[1])
redis.call('DEL', KEYS[2])
redis.call('SET', KEYS[3], '1', 'PX', ARGV[1])
return 1
"#;

/// Read from L2.
/// KEYS\[1\] = data key
/// Returns {data_json, pttl} on hit, nil on miss.
///
/// Prefix invalidation is L1-only (within a process and across instances via
/// pub/sub); L2 entries rely on TTL for prefix-scoped cleanup, so this script
/// has no generation check.
const L2_READ: &str = r#"
local data = redis.call('GET', KEYS[1])
if not data then
  return nil
end
local pttl = redis.call('PTTL', KEYS[1])
return {data, pttl}
"#;

pub(crate) struct Scripts {
    pub lease_acquire: Script,
    pub lease_cas_write: Script,
    pub lease_release: Script,
    pub tombstone_guarded_insert: Script,
    pub invalidate_key: Script,
    pub l2_read: Script,
}

impl Scripts {
    pub fn new() -> Self {
        Self {
            lease_acquire: Script::from_lua(LEASE_ACQUIRE),
            lease_cas_write: Script::from_lua(LEASE_CAS_WRITE),
            lease_release: Script::from_lua(LEASE_RELEASE),
            tombstone_guarded_insert: Script::from_lua(TOMBSTONE_GUARDED_INSERT),
            invalidate_key: Script::from_lua(INVALIDATE_KEY),
            l2_read: Script::from_lua(L2_READ),
        }
    }
}
