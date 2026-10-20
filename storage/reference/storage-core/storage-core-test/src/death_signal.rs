// SPDX-FileCopyrightText: Copyright (c) 2024 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub trait DeathSignal {
    fn death_signal(&mut self);
}

#[cfg(target_os = "linux")]
impl DeathSignal for std::process::Command {
    fn death_signal(&mut self) {
        use std::io;
        use std::os::unix::process::CommandExt;
        let ppid_before_fork = std::process::id();
        unsafe {
            self.pre_exec(move || {
                nix::sys::prctl::set_pdeathsig(nix::sys::signal::Signal::SIGTERM).unwrap();

                // test in case the original parent exited just before the prctl() call
                if std::os::unix::process::parent_id() != ppid_before_fork {
                    return Err(io::Error::other("parent pid changed after fork!"));
                }

                Ok(())
            });
        }
    }
}
