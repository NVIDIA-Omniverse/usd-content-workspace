// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::info_limited;
use tokio::signal;

pub async fn handle_signals() -> Result<&'static str, Box<dyn std::error::Error>> {
    use std::io::{self, IsTerminal};

    let mut sig_int = signal::unix::signal(signal::unix::SignalKind::interrupt())?;
    let mut sig_term = signal::unix::signal(signal::unix::SignalKind::terminate())?;

    info_limited!(is_terminal = io::stdin().is_terminal());

    loop {
        tokio::select! {
            _ = sig_int.recv() => {
                if io::stdin().is_terminal() {
                    return Ok("SIGINT")
                } else {
                    info_limited!("SIGINT received, terminal not detected, ignoring")
                }
            },
            _ = sig_term.recv() => {
                return Ok("SIGTERM")
            },
        }
    }
}
