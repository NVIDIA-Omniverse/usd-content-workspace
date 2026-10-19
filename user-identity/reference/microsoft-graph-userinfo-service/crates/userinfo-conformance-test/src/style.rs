// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
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

//! Console styling helpers for the conformance test.
//!
//! Wraps a small set of ANSI escape codes for color and bold/dim emphasis,
//! plus box-drawing helpers for banners, section headers, and step
//! separators. All styling is gated on a one-time check that respects:
//!
//! - `NO_COLOR=1` (or any non-empty value) → disable all styling, emit plain
//!   text. Standard agreed-upon convention; see <https://no-color.org/>.
//! - `FORCE_COLOR=1` → force styling on, even when stdout is not a TTY.
//! - Otherwise styling is enabled iff stdout is a terminal.
//!
//! When styling is disabled, every helper falls back to plain text, so the
//! same call sites work in CI logs and on a developer terminal.

use std::io::IsTerminal;
use std::sync::OnceLock;

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const CYAN: &str = "\x1b[36m";
const BOLD_RED: &str = "\x1b[1;31m";
const BOLD_GREEN: &str = "\x1b[1;32m";
const BOLD_CYAN: &str = "\x1b[1;36m";

const SECTION_WIDTH: usize = 76;

fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        if std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
            return false;
        }
        if let Ok(v) = std::env::var("FORCE_COLOR")
            && !v.is_empty()
            && v != "0"
        {
            return true;
        }
        std::io::stdout().is_terminal()
    })
}

fn paint(prefix: &str, text: &str) -> String {
    if enabled() {
        format!("{prefix}{text}{RESET}")
    } else {
        text.to_string()
    }
}

pub fn dim(text: &str) -> String {
    paint(DIM, text)
}

pub fn cyan(text: &str) -> String {
    paint(CYAN, text)
}

pub fn bold(text: &str) -> String {
    paint(BOLD, text)
}

pub fn bold_red(text: &str) -> String {
    paint(BOLD_RED, text)
}

pub fn bold_green(text: &str) -> String {
    paint(BOLD_GREEN, text)
}

pub fn bold_cyan(text: &str) -> String {
    paint(BOLD_CYAN, text)
}

/// Returns the styled `PASS` marker used at the start of a passed test line.
pub fn pass_marker() -> &'static str {
    static M: OnceLock<&'static str> = OnceLock::new();
    M.get_or_init(|| {
        if enabled() {
            leak(format!("{BOLD_GREEN}\u{2714} PASS{RESET}"))
        } else {
            "PASS"
        }
    })
}

/// Returns the styled `FAIL` marker used at the start of a failed test line.
pub fn fail_marker() -> &'static str {
    static M: OnceLock<&'static str> = OnceLock::new();
    M.get_or_init(|| {
        if enabled() {
            leak(format!("{BOLD_RED}\u{2718} FAIL{RESET}"))
        } else {
            "FAIL"
        }
    })
}

/// Returns the styled `INFO` marker used at the start of an informational
/// line such as readiness probe progress or fixture-sample messages.
pub fn info_marker() -> &'static str {
    static M: OnceLock<&'static str> = OnceLock::new();
    M.get_or_init(|| {
        if enabled() {
            leak(format!("{CYAN}\u{2139} INFO{RESET}"))
        } else {
            "INFO"
        }
    })
}

/// Renders the top-of-run banner with a double-line border. The title is
/// centered horizontally inside the box.
pub fn print_banner(title: &str) {
    let inner = SECTION_WIDTH;
    let title_chars = title.chars().count();
    let pad_total = inner.saturating_sub(title_chars + 2);
    let pad_left = pad_total / 2;
    let pad_right = pad_total - pad_left;
    let top = format!("\u{2554}{}\u{2557}", "\u{2550}".repeat(inner));
    let mid = format!(
        "\u{2551} {}{}{} \u{2551}",
        " ".repeat(pad_left),
        title,
        " ".repeat(pad_right),
    );
    let bot = format!("\u{255A}{}\u{255D}", "\u{2550}".repeat(inner));
    println!();
    println!("{}", bold_cyan(&top));
    println!("{}", bold_cyan(&mid));
    println!("{}", bold_cyan(&bot));
}

/// Prints a top-level step header preceded and followed by a horizontal
/// rule. Used to separate phases (pre-flight, fixture sampling, REST
/// conformance, gRPC conformance, summary).
pub fn print_section(title: &str) {
    let bar: String = "\u{2501}".repeat(SECTION_WIDTH);
    println!();
    println!("{}", cyan(&bar));
    println!("{} {}", cyan("\u{25B6}"), bold_cyan(title));
    println!("{}", cyan(&bar));
}

/// Leaks a `String` into a `&'static str`. Used only for the marker getters
/// above, which initialise once per process and live for the whole run.
fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}
