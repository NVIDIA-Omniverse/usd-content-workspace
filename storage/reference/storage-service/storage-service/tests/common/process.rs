// SPDX-FileCopyrightText: Copyright (c) 2024 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

/// Extension trait that arms a child `Command` with `PR_SET_PDEATHSIG(SIGTERM)`, so that
/// spawned `storage-service` test processes are automatically killed with `SIGTERM` when
/// the test runner process exits — even if it exits abnormally without running `Drop`.
pub trait DeathSignalExt {
    fn death_signal(&mut self);
}

#[cfg(target_os = "linux")]
impl DeathSignalExt for std::process::Command {
    fn death_signal(&mut self) {
        use std::io;
        use std::os::unix::process::CommandExt;
        let ppid_before_fork = std::process::id();
        unsafe {
            self.pre_exec(move || {
                use libc::{prctl, PR_SET_PDEATHSIG, SIGTERM};
                let result = prctl(PR_SET_PDEATHSIG, SIGTERM, 0, 0, 0);
                assert_eq!(result, 0);

                // test in case the original parent exited just before the prctl() call
                if std::os::unix::process::parent_id() != ppid_before_fork {
                    return Err(io::Error::other("parent pid changed after fork!"));
                }

                Ok(())
            });
        }
    }
}

// On non-Linux platforms (darwin local dev) PR_SET_PDEATHSIG isn't available, so the auto-kill
// safety net is a no-op — children rely on `ServiceHandle::Drop` for cleanup. If the test runner
// dies in a way that bypasses Drop, spawned `storage-service` children may leak and need manual
// cleanup.
#[cfg(not(target_os = "linux"))]
impl DeathSignalExt for std::process::Command {
    fn death_signal(&mut self) {}
}
