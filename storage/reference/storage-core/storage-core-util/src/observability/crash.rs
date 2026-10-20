// SPDX-FileCopyrightText: Copyright (c) 2024 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    fmt,
    io::Write,
    panic,
    sync::atomic::{self, AtomicBool},
};

use crate::error_limited;

/// There is no `Display` for `Option<Location>`, but we need that to package it into a single trace output.
struct MaybeLocation<'a>(Option<&'a panic::Location<'a>>);

impl fmt::Display for MaybeLocation<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if let Some(location) = self.0 {
            write!(f, "{location}")
        } else {
            write!(f, "<unknown location>")
        }
    }
}

fn handle_panic(info: &panic::PanicHookInfo) {
    const PANIC_EXIT_CODE: i32 = 101;
    static IS_PANICKING: AtomicBool = AtomicBool::new(false);
    if IS_PANICKING.swap(true, atomic::Ordering::SeqCst) {
        // Protect ourselves from infinite recursion in panic. Not much we can do here because it might be the logging
        // system itself that is causing the panic.
        eprintln!("!!! PANIC called while we were already panicking -- exiting immediately !!!");
        std::process::exit(PANIC_EXIT_CODE);
    }

    let location = MaybeLocation(info.location());
    // When `payload_as_str` is stabilized, that should be used instead. As of Rust 2021, a `panic!` will always
    // have an associated `&str`, but the fallback is here in case someone uses `panic_any` with a non-string.
    let message = info
        .payload()
        .downcast_ref::<&str>()
        .map_or("<no associated message>", |msg| *msg);

    // Always capture a backtrace when panicking -- the expense does not matter since we're crashing anyway
    let backtrace = std::backtrace::Backtrace::force_capture();

    // First, log out to STDOUT. This protects us in the case of the tracing system being the thing causing the panic.
    // At the very least, someone should be able to see "PANIC" in the output.
    println!("!!! PANIC called at {location}: {message}\n\n{backtrace}");
    let _ = std::io::stdout().flush();

    error_limited!(
        %location,
        %message,
        "PANIC called at {location}: {message}\n\n{backtrace}"
    );

    std::process::exit(PANIC_EXIT_CODE);
}

/// Install a global panic handler that attempts to output stacks to tracing.
///
/// This is called automatically from `init_framework`, so you typically do not need to call it yourself.
pub fn install_panic_handler() {
    panic::set_hook(Box::new(handle_panic));
}
