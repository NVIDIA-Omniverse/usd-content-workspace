// SPDX-FileCopyrightText: Copyright (c) 2024-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

use super::{LogLine, Logs};
use crate::types::common::TimestampRange;
use std::fmt;

#[derive(Debug)]
pub struct LogParseError<'a> {
    line_num: usize,
    line: &'a str,
    source: serde_json::Error,
}

impl<'a> LogParseError<'a> {
    pub fn line_num(&self) -> usize {
        self.line_num
    }

    pub fn line(&self) -> &'a str {
        self.line
    }
}

impl fmt::Display for LogParseError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {} is invalid: {}", self.line_num, self.source)
    }
}

impl std::error::Error for LogParseError<'_> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

impl<'a> Logs<'a> {
    pub fn parse(lines: &'a str) -> Result<Self, LogParseError<'a>> {
        lines
            .trim_matches('\n')
            .lines()
            .enumerate()
            .map(|(line_num, line)| {
                serde_json::from_str(line).map_err(|source| LogParseError {
                    line_num,
                    line,
                    source,
                })
            })
            .collect::<Result<Vec<LogLine>, LogParseError<'_>>>()
            .map(|lines| Self { lines })
    }

    #[track_caller]
    pub fn assert_timestamps(&mut self) {
        let timestamp = TimestampRange::now();
        for log in &mut self.lines {
            assert_eq!(
                timestamp, log.timestamp,
                "log time {} < {} < {}: {:#?}",
                timestamp.start, log.timestamp, timestamp.end, log
            );
            log.timestamp = 0;
        }
    }

    #[track_caller]
    pub fn assert_line_numbers(&mut self) {
        for log in &mut self.lines {
            if matches!(log.line, Some(0) | None) {
                panic!("log line {:?} > 0: {:#?}", log.line, log);
            }
            log.line = None;
        }
    }
}
