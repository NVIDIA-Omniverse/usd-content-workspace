// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

//! What one generate message resulted in, as a value.
//!
//! The pipeline produces this and stops. Deciding whether to acknowledge is a
//! separate concern in `consumer.rs`, and labelling a metric is another in
//! `metrics.rs`. Keeping classification out of the acknowledgement path is what
//! lets the synchronous mode map the same outcomes onto HTTP status codes later
//! without re-deriving any of it.

use std::fmt;

/// Which stage of the pipeline failed, used as a metric label and in the log
/// line that precedes every permanent acknowledgement.
///
/// One variant per stage that can fail, deliberately. A class that absorbs
/// several stages tells an operator only that something went wrong somewhere in
/// the middle, which is the visibility the metric exists to provide in the first
/// place now that there is no task table to query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    /// The message did not satisfy the generate contract.
    Validation,
    /// The message asked for a processor this build does not implement.
    UnsupportedProcessor,
    /// No outbound credential could be obtained. Nothing to do with the message
    /// or its source: every conversion fails this way while it lasts.
    Credential,
    /// The source object does not exist.
    SourceMissing,
    /// The source exists but could not be transferred out of storage.
    Download,
    /// No decoder handles the source format.
    UnsupportedFormat,
    /// The source bytes could not be decoded, or exceeded a decode bound.
    Decode,
    /// The thumbnail could not be written in the requested output format.
    Encode,
    /// The thumbnail could not be written to its destination.
    Upload,
    /// thumbnail-service did not record the association.
    Associate,
}

impl FailureClass {
    /// Whether a failure of this class happened after decoding began.
    ///
    /// Used to decide whether a delivery's elapsed time belongs in the conversion
    /// latency distribution. The classes before this line reject the message or
    /// its source without decoding anything, and they are fast — so counting them
    /// as conversion latency pulls the distribution down systematically rather
    /// than adding noise.
    fn is_after_conversion_started(self) -> bool {
        match self {
            Self::Validation
            | Self::UnsupportedProcessor
            | Self::Credential
            | Self::SourceMissing
            | Self::Download
            | Self::UnsupportedFormat => false,
            Self::Decode | Self::Encode | Self::Upload | Self::Associate => true,
        }
    }
}

impl FailureClass {
    /// The stable string used as a metric label. Kept explicit rather than
    /// derived from the variant name so a rename cannot silently break a
    /// dashboard.
    pub fn as_label(self) -> &'static str {
        match self {
            Self::Validation => "validation",
            Self::UnsupportedProcessor => "unsupported_processor",
            Self::Credential => "credential",
            Self::SourceMissing => "source_missing",
            Self::Download => "download",
            Self::UnsupportedFormat => "unsupported_format",
            Self::Decode => "decode",
            Self::Encode => "encode",
            Self::Upload => "upload",
            Self::Associate => "associate",
        }
    }

    /// Every class, in pipeline order.
    ///
    /// This is the one list; counters are stored by position in it and metric
    /// rendering walks it, so a class cannot exist without being reported. The
    /// guard against forgetting to add one is
    /// `tests::every_variant_is_listed_in_all`, whose match is exhaustive: a new
    /// variant fails to compile there until it is named, and naming it without
    /// adding it here fails the assertion.
    pub const ALL: [FailureClass; 10] = [
        Self::Validation,
        Self::UnsupportedProcessor,
        Self::Credential,
        Self::SourceMissing,
        Self::Download,
        Self::UnsupportedFormat,
        Self::Decode,
        Self::Encode,
        Self::Upload,
        Self::Associate,
    ];

    /// Position in [`FailureClass::ALL`], used to index the counter array.
    ///
    /// Derived from the list rather than written out a second time, so the two
    /// cannot disagree about which slot a class owns.
    pub fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|class| *class == self)
            .expect("every failure class is listed in FailureClass::ALL")
    }
}

impl fmt::Display for FailureClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_label())
    }
}

/// The classified result of processing one generate message.
#[derive(Debug)]
pub enum WorkOutcome {
    /// Converted, uploaded, and associated.
    Succeeded { destination: String },
    /// The same input will fail the same way. Acknowledge it, after recording it.
    Permanent {
        class: FailureClass,
        destination: String,
        detail: String,
    },
    /// A dependency failed. Withhold the acknowledgement so it comes back.
    Retryable {
        class: FailureClass,
        destination: String,
        detail: String,
    },
}

impl WorkOutcome {
    pub fn permanent(
        class: FailureClass,
        destination: impl Into<String>,
        detail: impl fmt::Display,
    ) -> Self {
        Self::Permanent {
            class,
            destination: destination.into(),
            detail: detail.to_string(),
        }
    }

    pub fn retryable(
        class: FailureClass,
        destination: impl Into<String>,
        detail: impl fmt::Display,
    ) -> Self {
        Self::Retryable {
            class,
            destination: destination.into(),
            detail: detail.to_string(),
        }
    }

    /// Whether the delivery carrying this message may be acknowledged.
    ///
    /// Success and permanent rejection both acknowledge: a poison message that
    /// kept its delivery would tight-loop forever and never reach the
    /// dead-letter queue either, because the delivery counter only advances on
    /// an actual redelivery.
    pub fn should_ack(&self) -> bool {
        !matches!(self, Self::Retryable { .. })
    }

    /// Whether this delivery got far enough for its elapsed time to be
    /// conversion latency.
    ///
    /// A success obviously converted. A failure counts only if it happened at or
    /// after decode: everything earlier rejects the message or its source without
    /// decoding, and timing those as conversions makes the distribution report a
    /// service faster than it is.
    pub fn reached_conversion(&self) -> bool {
        match self {
            Self::Succeeded { .. } => true,
            Self::Permanent { class, .. } | Self::Retryable { class, .. } => {
                class.is_after_conversion_started()
            }
        }
    }

    pub fn failure_class(&self) -> Option<FailureClass> {
        match self {
            Self::Succeeded { .. } => None,
            Self::Permanent { class, .. } | Self::Retryable { class, .. } => Some(*class),
        }
    }

    pub fn destination(&self) -> &str {
        match self {
            Self::Succeeded { destination }
            | Self::Permanent { destination, .. }
            | Self::Retryable { destination, .. } => destination,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_and_permanent_rejection_both_acknowledge() {
        assert!(WorkOutcome::Succeeded {
            destination: "https://storage.example/thumb.png".to_string(),
        }
        .should_ack());

        assert!(
            WorkOutcome::permanent(FailureClass::Validation, "dest", "missing field").should_ack(),
            "poison must be acknowledged or it tight-loops without ever dead-lettering"
        );
    }

    #[test]
    fn a_retryable_failure_withholds_the_acknowledgement() {
        assert!(
            !WorkOutcome::retryable(FailureClass::Upload, "dest", "storage unavailable")
                .should_ack()
        );
    }

    /// Metrics read the class off the outcome rather than inferring it from
    /// whether an ACK was sent, so the two cannot disagree.
    #[test]
    fn the_failure_class_travels_with_the_outcome() {
        let outcome = WorkOutcome::permanent(FailureClass::UnsupportedFormat, "dest", "heic");

        assert_eq!(
            outcome.failure_class(),
            Some(FailureClass::UnsupportedFormat)
        );
        assert_eq!(
            outcome.failure_class().unwrap().as_label(),
            "unsupported_format"
        );
        assert_eq!(
            WorkOutcome::Succeeded {
                destination: "dest".to_string()
            }
            .failure_class(),
            None
        );
    }

    #[test]
    fn every_class_has_a_distinct_stable_label() {
        let labels = FailureClass::ALL.map(FailureClass::as_label);

        let mut unique = labels.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), labels.len(), "labels must be distinguishable");
    }

    /// The guard that keeps `ALL` honest. The match is exhaustive, so a new
    /// variant does not compile until it is named here; `index` then panics
    /// unless it was also added to `ALL`, which is what counters and metric
    /// rendering both walk.
    #[test]
    fn every_variant_is_listed_in_all() {
        let every_variant = [
            FailureClass::Validation,
            FailureClass::UnsupportedProcessor,
            FailureClass::Credential,
            FailureClass::SourceMissing,
            FailureClass::Download,
            FailureClass::UnsupportedFormat,
            FailureClass::Decode,
            FailureClass::Encode,
            FailureClass::Upload,
            FailureClass::Associate,
        ];

        for class in every_variant {
            // Exhaustive on purpose: this is the line that fails to compile
            // when a variant is added.
            match class {
                FailureClass::Validation
                | FailureClass::UnsupportedProcessor
                | FailureClass::Credential
                | FailureClass::SourceMissing
                | FailureClass::Download
                | FailureClass::UnsupportedFormat
                | FailureClass::Decode
                | FailureClass::Encode
                | FailureClass::Upload
                | FailureClass::Associate => {}
            }
            // Panics if the variant is missing from `ALL`.
            let _ = class.index();
        }

        assert_eq!(FailureClass::ALL.len(), every_variant.len());
    }

    /// Each class owns a distinct counter slot, or two stages would share a
    /// series and the split would be cosmetic.
    #[test]
    fn class_indices_are_distinct_and_dense() {
        let mut indices: Vec<usize> = FailureClass::ALL.iter().map(|c| c.index()).collect();
        indices.sort_unstable();

        assert_eq!(indices, (0..FailureClass::ALL.len()).collect::<Vec<_>>());
    }
}
