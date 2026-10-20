// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::{Mutex, OnceLock};
use std::thread::JoinHandle;
use tokio::sync::mpsc;

pub static THREAD_REGISTRY: OnceLock<Mutex<ThreadRegistry>> = OnceLock::new();

pub struct ThreadRegistry {
    threads: Vec<(JoinHandle<()>, mpsc::Sender<()>)>,
}

impl ThreadRegistry {
    fn new() -> Self {
        Self { threads: Vec::new() }
    }

    pub fn register_thread(&mut self, handle: JoinHandle<()>, shutdown_tx: mpsc::Sender<()>) {
        self.threads.push((handle, shutdown_tx));
    }

    fn shutdown_all(&mut self) {
        for (handle, shutdown_tx) in self.threads.drain(..) {
            let _ = shutdown_tx.try_send(()); // Signal shutdown
            let _ = handle.join(); // Wait for thread to complete
        }
    }
}

/// Initialize the signal handler
///
/// This function will set up the Ctrl-C handler to shutdown all registered threads.
///
/// This function should be called before any threads are spawned.
pub fn init_signal_handler() {
    // Initialize thread registry
    THREAD_REGISTRY.get_or_init(|| Mutex::new(ThreadRegistry::new()));

    // Set up signal handler (only once globally)
    static SIGNAL_INIT: std::sync::Once = std::sync::Once::new();
    SIGNAL_INIT.call_once(|| {
        ctrlc::set_handler(move || {
            // Shutdown all registered threads
            if let Some(registry) = THREAD_REGISTRY.get() {
                if let Ok(mut registry) = registry.lock() {
                    registry.shutdown_all();
                }
            }

            std::process::exit(130);
        })
        .expect("Failed to set Ctrl-C handler");
    });
}
