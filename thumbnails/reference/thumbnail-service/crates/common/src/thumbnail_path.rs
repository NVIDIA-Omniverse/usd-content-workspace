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

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VarKind {
    ResourcePath,
    ResourceFilename,
    ResourceAddressMd5,
    ResourceIdentityMd5,
    Width,
    Height,
    OutputEncoding,
    Quality,
}

impl VarKind {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "resource_path" => Self::ResourcePath,
            "resource_filename" => Self::ResourceFilename,
            "resource_address_md5" => Self::ResourceAddressMd5,
            "resource_identity_md5" => Self::ResourceIdentityMd5,
            "width" => Self::Width,
            "height" => Self::Height,
            "output_encoding" => Self::OutputEncoding,
            "quality" => Self::Quality,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Literal(String),
    Var(VarKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbnailPathTemplate {
    segments: Vec<Segment>,
}

#[derive(Debug, Clone, Copy)]
pub struct ThumbnailPathContext<'a> {
    pub resource_address: &'a str,
    pub resource_identity: &'a str,
    pub width: u32,
    pub height: u32,
    pub output_encoding: &'a str,
    pub quality: u32,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ThumbnailPathError {
    #[error("thumbnail path template has malformed brace syntax")]
    MalformedBraces,
    #[error("thumbnail path template references unknown variable '{0}'")]
    UnknownVariable(String),
    #[error(
        "thumbnail path template must identify its source with \
         '{{resource_address_md5}}' or both '{{resource_path}}' and '{{resource_filename}}'"
    )]
    MissingSourceIdentity,
    #[error("thumbnail path template must render an absolute storage address: {0}")]
    NonAbsoluteAddress(String),
    #[error("source resource address must be an absolute storage address: {0}")]
    InvalidSourceAddress(String),
    #[error(
        "thumbnail path template does not recognize its own rendered output '{0}': \
         the template likely has adjacent greedy-string variables \
         ({{{{resource_path}}}} or {{{{resource_filename}}}}) or adjacent numeric variables \
         ({{{{width}}}} or {{{{height}}}}) with no literal separator between them — \
         add a path separator or fixed suffix (e.g. 'x' between width/height, '.thumbs/' \
         between path segments) between them"
    )]
    FailsRoundTrip(String),
}

impl ThumbnailPathTemplate {
    pub fn parse(raw: impl Into<String>) -> Result<Self, ThumbnailPathError> {
        let raw = raw.into();
        let segments = tokenize(&raw)?;
        let has_variable = |kind| {
            segments
                .iter()
                .any(|segment| matches!(segment, Segment::Var(value) if *value == kind))
        };

        // Without a variable that identifies *which* source produced this output,
        // two different sources can render to the identical destination and
        // silently overwrite each other's thumbnail — e.g. a template like
        // "{width}x{height}/thumb.{output_encoding}" renders the same path for
        // every source of a given size. Requiring {resource_address_md5} (or the
        // {resource_path}+{resource_filename} pair) guarantees the rendered path
        // is unique per source. See ADR 0001 (docs/adr/0001-thumbnail-storage.md)
        // for the full collision/race-condition rationale.
        let identifies_source = has_variable(VarKind::ResourceAddressMd5)
            || (has_variable(VarKind::ResourcePath) && has_variable(VarKind::ResourceFilename));
        if !identifies_source {
            return Err(ThumbnailPathError::MissingSourceIdentity);
        }

        let template = Self { segments };
        let smoke_context = ThumbnailPathContext {
            resource_address: "https://storage.example/path/source.png",
            resource_identity: "https://storage.example/path/source.png?version=1",
            width: 256,
            height: 256,
            output_encoding: "PNG",
            quality: 85,
        };
        let rendered = template.render(&smoke_context)?;

        // Reject templates whose rendered output cannot be recognized by the
        // structural matcher (`matches_rendered_output`), because that matcher
        // IS the active recursion guard: it is the only mechanism today that
        // prevents the service from thumbnailing its own generated output
        // indefinitely.  A template that renders correctly but then can't
        // round-trip through the matcher silently disables the guard.
        //
        // The most common root cause is adjacent greedy-string variables with
        // no literal separator (e.g. `{resource_path}{resource_filename}` and
        // `{width}{height}`): `rfind`-based matching over-consumes through the
        // downstream separators, leaving the second variable with nothing to match.
        if !template.matches_rendered_output(&rendered) {
            return Err(ThumbnailPathError::FailsRoundTrip(rendered));
        }

        Ok(template)
    }

    /// Returns true when `address` matches the structural shape of a path this
    /// template can render (literals plus constrained variable segments).
    ///
    /// Used to skip storage CREATE events for generated thumbnails when the
    /// event pipeline does not yet propagate writer principal or pipeline markers.
    pub fn matches_rendered_output(&self, address: &str) -> bool {
        // Strip query/fragment via a real URL parser rather than splitting on the
        // first raw '?'/'#' byte (MR #153 review, Artem): `render()` already
        // parses addresses this way (see `resource_path()`), and a naive byte
        // split can misidentify where the query starts if a path segment ever
        // contains an unencoded '?' or '#'. A percent-encoded '?' (`%3F`) in the
        // path is unaffected either way, since it contains no raw '?' byte.
        // Addresses that fail to parse as a URL fall back to the raw string,
        // which still fails to structurally match (`validate_destination_address`
        // guarantees every real rendered destination is a parseable absolute URL).
        let stripped = match reqwest::Url::parse(address) {
            Ok(mut parsed) => {
                parsed.set_query(None);
                parsed.set_fragment(None);
                parsed.to_string()
            }
            Err(_) => address.to_string(),
        };
        let mut remainder = stripped.as_str();

        for (index, segment) in self.segments.iter().enumerate() {
            let next_literal = self.segments[index + 1..].iter().find_map(|segment| {
                if let Segment::Literal(literal) = segment {
                    Some(literal.as_str())
                } else {
                    None
                }
            });

            match segment {
                Segment::Literal(literal) => {
                    if !remainder.starts_with(literal) {
                        return false;
                    }
                    remainder = &remainder[literal.len()..];
                }
                Segment::Var(kind) => {
                    let Some(rest) = consume_variable_segment(remainder, *kind, next_literal)
                    else {
                        return false;
                    };
                    remainder = rest;
                }
            }
        }

        remainder.is_empty()
    }

    pub fn render(&self, context: &ThumbnailPathContext<'_>) -> Result<String, ThumbnailPathError> {
        let source = reqwest::Url::parse(context.resource_address).map_err(|_| {
            ThumbnailPathError::InvalidSourceAddress(context.resource_address.to_string())
        })?;
        if !source.has_host() {
            return Err(ThumbnailPathError::InvalidSourceAddress(
                context.resource_address.to_string(),
            ));
        }

        let uses_variable = |kind| {
            self.segments
                .iter()
                .any(|segment| matches!(segment, Segment::Var(value) if *value == kind))
        };

        // resource_path/resource_filename (URL path-segment parsing) and the MD5
        // digests are the only non-trivial per-render work; skip computing them
        // when the configured template doesn't reference the corresponding
        // variable (MR #153 review, Artem).
        let needs_filename =
            uses_variable(VarKind::ResourceFilename) || uses_variable(VarKind::ResourcePath);
        let (resource_filename, resource_path) = if needs_filename {
            let resource_filename = source
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                .filter(|filename| !filename.is_empty())
                .ok_or_else(|| {
                    ThumbnailPathError::InvalidSourceAddress(context.resource_address.to_string())
                })?
                .to_string();
            let resource_path = resource_path(&source, &resource_filename);
            (resource_filename, resource_path)
        } else {
            (String::new(), String::new())
        };

        let resource_address_md5 = if uses_variable(VarKind::ResourceAddressMd5) {
            format!("{:x}", md5::compute(context.resource_address))
        } else {
            String::new()
        };
        let resource_identity_md5 = if uses_variable(VarKind::ResourceIdentityMd5) {
            format!("{:x}", md5::compute(context.resource_identity))
        } else {
            String::new()
        };
        let width = context.width.to_string();
        let height = context.height.to_string();
        let output_encoding = context.output_encoding.to_lowercase();
        let quality = context.quality.to_string();

        let mut rendered = String::new();
        for segment in &self.segments {
            let value = match segment {
                Segment::Literal(value) => value,
                Segment::Var(VarKind::ResourcePath) => &resource_path,
                Segment::Var(VarKind::ResourceFilename) => &resource_filename,
                Segment::Var(VarKind::ResourceAddressMd5) => &resource_address_md5,
                Segment::Var(VarKind::ResourceIdentityMd5) => &resource_identity_md5,
                Segment::Var(VarKind::Width) => &width,
                Segment::Var(VarKind::Height) => &height,
                Segment::Var(VarKind::OutputEncoding) => &output_encoding,
                Segment::Var(VarKind::Quality) => &quality,
            };

            rendered.push_str(value);
        }

        validate_destination_address(&rendered)?;

        // Reparse and reserialize through the same URL library that
        // `matches_rendered_output` uses to normalize event addresses (MR #153
        // review, Mitri), so this function's return value is *already* in the
        // canonical form recursion prevention will compare a storage CREATE-event
        // address against, rather than a raw literal+variable concatenation that
        // was never round-tripped through a URL parser. For canonical-syntax
        // templates (the only kind used in practice — copied verbatim from a
        // cloud storage endpoint) this is a no-op; it only changes behavior for
        // template literals with non-canonical syntax (e.g. a redundant default
        // port or mixed-case host), which would otherwise be preserved verbatim
        // here while `matches_rendered_output` strips/lowercases the equivalent
        // in its input. Note this normalizes `render()`'s *output*, not the
        // template's literal segments used for matching — see the residual-scope
        // note on `render_output_is_already_url_normalized` for what remains.
        let normalized = reqwest::Url::parse(&rendered)
            .expect("validate_destination_address above already confirmed this parses")
            .to_string();
        Ok(normalized)
    }
}

impl std::str::FromStr for ThumbnailPathTemplate {
    type Err = ThumbnailPathError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

pub fn validate_destination_address(address: &str) -> Result<(), ThumbnailPathError> {
    let destination = reqwest::Url::parse(address)
        .map_err(|_| ThumbnailPathError::NonAbsoluteAddress(address.to_string()))?;
    if !destination.has_host() {
        return Err(ThumbnailPathError::NonAbsoluteAddress(address.to_string()));
    }
    Ok(())
}

fn tokenize(template: &str) -> Result<Vec<Segment>, ThumbnailPathError> {
    let mut segments = Vec::new();
    let mut cursor = 0;

    while let Some(relative_brace) = template[cursor..].find(['{', '}']) {
        let brace = cursor + relative_brace;
        if template.as_bytes()[brace] == b'}' {
            return Err(ThumbnailPathError::MalformedBraces);
        }
        if brace > cursor {
            segments.push(Segment::Literal(template[cursor..brace].to_string()));
        }

        let from_brace = &template[brace..];
        let relative_close = from_brace
            .find('}')
            .ok_or(ThumbnailPathError::MalformedBraces)?;
        let variable = &from_brace[1..relative_close];
        if variable.is_empty() || variable.contains(['{', '}']) {
            return Err(ThumbnailPathError::MalformedBraces);
        }
        let kind = VarKind::from_name(variable)
            .ok_or_else(|| ThumbnailPathError::UnknownVariable(variable.to_string()))?;
        segments.push(Segment::Var(kind));
        cursor = brace + relative_close + 1;
    }

    if cursor < template.len() {
        segments.push(Segment::Literal(template[cursor..].to_string()));
    }

    Ok(segments)
}

fn consume_variable_segment<'a>(
    remainder: &'a str,
    kind: VarKind,
    next_literal: Option<&str>,
) -> Option<&'a str> {
    match kind {
        VarKind::ResourceAddressMd5 | VarKind::ResourceIdentityMd5 => take_fixed_hex32(remainder),
        VarKind::Width | VarKind::Height | VarKind::Quality => take_digits(remainder),
        VarKind::OutputEncoding => take_output_encoding(remainder),
        VarKind::ResourceFilename => take_until_literal(remainder, next_literal, true),
        VarKind::ResourcePath => take_until_literal(remainder, next_literal, false),
    }
}

fn take_fixed_hex32(remainder: &str) -> Option<&str> {
    let candidate = remainder.get(..32)?;
    if candidate.len() != 32 || !candidate.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    Some(&remainder[32..])
}

fn take_digits(remainder: &str) -> Option<&str> {
    let digit_len = remainder
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .count();
    if digit_len == 0 {
        return None;
    }
    Some(&remainder[digit_len..])
}

fn take_output_encoding(remainder: &str) -> Option<&str> {
    const OUTPUT_ENCODINGS: &[&str] = &[
        "jpeg", "webp", "heic", "heif", "avif", "tiff", "jpg", "png", "gif", "bmp", "tif",
    ];

    for encoding in OUTPUT_ENCODINGS {
        if let Some(rest) = remainder.strip_prefix(encoding) {
            return Some(rest);
        }
    }

    None
}

fn take_until_literal<'a>(
    remainder: &'a str,
    next_literal: Option<&str>,
    require_non_empty: bool,
) -> Option<&'a str> {
    let (matched, rest) = if let Some(literal) = next_literal {
        // Rightmost, not leftmost: `resource_path`/`resource_filename` are
        // greedy, source-controlled text that can itself contain the
        // following literal (a real filename like `image.jpg` contains `.`,
        // which collides with a template separator like `{resource_filename}.
        // {output_encoding}`). The template-authored delimiter is always the
        // *last* occurrence, since everything after it is fixed-vocabulary
        // (digits, a known encoding, or another literal) that render() never
        // lets collide with `literal` again later in the string.
        let boundary = remainder.rfind(literal)?;
        if require_non_empty && boundary == 0 {
            return None;
        }
        (&remainder[..boundary], &remainder[boundary..])
    } else if require_non_empty && !remainder.is_empty() && !remainder.contains('/') {
        (remainder, "")
    } else {
        return None;
    };

    if require_non_empty && matched.is_empty() {
        return None;
    }

    let _ = matched;
    Some(rest)
}

/// The source's directory, with **no** trailing slash.
///
/// Templates are concatenated verbatim, so every separator in a rendered address
/// is one the template author wrote. Returning the directory with its slash
/// attached would break that: `{resource_path}/.thumbs/` — the way an operator
/// naturally writes it — rendered `/path//.thumbs/`, a different storage address
/// than the intended `/path/.thumbs/`, while the working spelling was the
/// counter-intuitive `{resource_path}.thumbs/`. Withholding the slash keeps the
/// rule simple enough to state in one line: what you write is what you get.
fn resource_path(source: &reqwest::Url, resource_filename: &str) -> String {
    let mut directory = source.clone();
    directory.set_query(None);
    directory.set_fragment(None);

    let directory_path =
        directory.path()[..directory.path().len() - resource_filename.len()].to_string();
    directory.set_path(&directory_path);

    // Trim on the serialized form, not via `set_path`: a URL with an http(s) scheme
    // always serializes at least a "/" path, so a source at the container root
    // (`https://host/file.jpg`) would keep its slash and double the template's own.
    // Exactly one separator is removed, so a source path that genuinely contains an
    // empty segment keeps it.
    let serialized = directory.to_string();
    serialized
        .strip_suffix('/')
        .unwrap_or(&serialized)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    fn context<'a>(
        resource_address: &'a str,
        resource_identity: &'a str,
    ) -> ThumbnailPathContext<'a> {
        ThumbnailPathContext {
            resource_address,
            resource_identity,
            width: 256,
            height: 128,
            output_encoding: "PNG",
            quality: 85,
        }
    }

    #[test]
    fn parse_tokenizes_adjacent_and_repeated_variables_once() {
        // The tokenizer must produce separate Var segments for adjacent variables
        // (no literal between them) and handle variables that appear more than once.
        // Only fixed-length variables ({resource_address_md5}, {resource_identity_md5})
        // can be adjacent and still round-trip through matches_rendered_output;
        // greedy-string ({resource_path}/{resource_filename}) and numeric ({width}/{height})
        // variables without a literal separator between them are rejected at parse time —
        // see adjacent_greedy_string_variables_without_separator_fail_round_trip and
        // adjacent_numeric_variables_without_separator_are_rejected_at_parse_time.
        let template = ThumbnailPathTemplate::parse(
            "https://thumbs.example/{resource_address_md5}{resource_identity_md5}/{width}x{height}/{width}.png",
        )
        .expect("valid template — fixed-length hex32 vars can be adjacent");

        assert_eq!(
            template.segments,
            vec![
                Segment::Literal("https://thumbs.example/".to_string()),
                Segment::Var(VarKind::ResourceAddressMd5),
                Segment::Var(VarKind::ResourceIdentityMd5),
                Segment::Literal("/".to_string()),
                Segment::Var(VarKind::Width),
                Segment::Literal("x".to_string()),
                Segment::Var(VarKind::Height),
                Segment::Literal("/".to_string()),
                Segment::Var(VarKind::Width),
                Segment::Literal(".png".to_string()),
            ]
        );
        let address_hash = format!(
            "{:x}",
            md5::compute("https://source.example/assets/image.jpg")
        );
        let identity_hash = format!(
            "{:x}",
            md5::compute("https://source.example/assets/image.jpg?version=1")
        );
        assert_eq!(
            template
                .render(&context(
                    "https://source.example/assets/image.jpg",
                    "https://source.example/assets/image.jpg?version=1",
                ))
                .expect("rendered destination"),
            format!("https://thumbs.example/{address_hash}{identity_hash}/256x128/256.png")
        );
    }

    #[test]
    fn adjacent_greedy_string_variables_without_separator_fail_round_trip() {
        // {resource_path} and {resource_filename} are both greedy-string variables.
        // When adjacent with no literal separator (e.g. `{resource_path}{resource_filename}-…`),
        // matches_rendered_output's rfind-based matching over-consumes through the
        // downstream separators (here `-`), leaving {resource_filename} with nothing
        // to match.  Such templates are rejected at parse() time so they cannot
        // silently disable the recursion guard — shape-matching is the only active
        // guard until storage propagates writer-principal signals.
        //
        // The fix for templates that need every variable: add a literal separator
        // (e.g. `/{resource_filename}` or `.{output_encoding}`) between the two
        // greedy-string variables.  See `documented_templates_all_round_trip`
        // for examples that do work.
        let err = ThumbnailPathTemplate::parse(
            "https://thumbs.example/{resource_address_md5}/{resource_identity_md5}/\
             {resource_path}{resource_filename}-{width}x{height}-{output_encoding}-{quality}",
        )
        .expect_err("adjacent greedy variables must fail the round-trip check");

        assert!(
            matches!(err, ThumbnailPathError::FailsRoundTrip(_)),
            "expected FailsRoundTrip, got: {err}"
        );
    }

    #[test]
    fn renders_nucleus_compatible_colocated_path() {
        let template = ThumbnailPathTemplate::parse(
            "{resource_path}/.thumbs/{width}x{height}/{resource_filename}.auto.{output_encoding}",
        )
        .expect("valid colocated template");
        let input = ThumbnailPathContext {
            height: 256,
            ..context(
                "https://foo.example/path/file.jpg",
                "https://foo.example/path/file.jpg?version=1",
            )
        };

        assert_eq!(
            template.render(&input).expect("rendered destination"),
            "https://foo.example/path/.thumbs/256x256/file.jpg.auto.png"
        );
    }

    /// Every separator in a rendered address is one the template author wrote.
    /// `{resource_path}` contributes no trailing slash of its own, so the number of
    /// slashes between it and what follows is exactly the number in the template —
    /// including none, and including a deliberate empty path segment.
    #[test]
    fn resource_path_joins_exactly_as_the_template_spells_it() {
        let input = ThumbnailPathContext {
            height: 256,
            ..context(
                "https://foo.example/path/file.jpg",
                "https://foo.example/path/file.jpg?version=1",
            )
        };

        for (template, expected) in [
            (
                "{resource_path}/.thumbs/{width}x{height}/{resource_filename}.auto.{output_encoding}",
                "https://foo.example/path/.thumbs/256x256/file.jpg.auto.png",
            ),
            (
                "{resource_path}//.thumbs/{width}x{height}/{resource_filename}.auto.{output_encoding}",
                "https://foo.example/path//.thumbs/256x256/file.jpg.auto.png",
            ),
            (
                "{resource_path}.thumbs/{width}x{height}/{resource_filename}.auto.{output_encoding}",
                "https://foo.example/path.thumbs/256x256/file.jpg.auto.png",
            ),
        ] {
            let rendered = ThumbnailPathTemplate::parse(template)
                .expect("valid colocated template")
                .render(&input)
                .expect("rendered destination");

            assert_eq!(rendered, expected, "template {template}");
        }
    }

    /// A source at the root of its container has no directory to speak of, and the
    /// template's own leading slash still has to produce a single separator.
    #[test]
    fn resource_path_at_the_container_root_does_not_double_the_separator() {
        let rendered = ThumbnailPathTemplate::parse(
            "{resource_path}/.thumbs/{width}x{height}/{resource_filename}.auto.{output_encoding}",
        )
        .expect("valid template")
        .render(&ThumbnailPathContext {
            height: 256,
            ..context(
                "https://foo.example/file.jpg",
                "https://foo.example/file.jpg?version=1",
            )
        })
        .expect("rendered destination");

        assert_eq!(
            rendered,
            "https://foo.example/.thumbs/256x256/file.jpg.auto.png"
        );
    }

    /// The separator collapse must not break the recursion guard: a template whose
    /// rendered output it can no longer recognize would silently stop skipping this
    /// service's own generated thumbnails.
    #[test]
    fn separator_forms_still_round_trip_through_the_recursion_matcher() {
        let input = ThumbnailPathContext {
            height: 256,
            ..context(
                "https://foo.example/path/file.jpg",
                "https://foo.example/path/file.jpg?version=1",
            )
        };

        for template in [
            "{resource_path}/.thumbs/{width}x{height}/{resource_filename}.auto.{output_encoding}",
            "{resource_path}//.thumbs/{width}x{height}/{resource_filename}.auto.{output_encoding}",
            "{resource_path}.thumbs/{width}x{height}/{resource_filename}.auto.{output_encoding}",
        ] {
            let parsed = ThumbnailPathTemplate::parse(template).expect("valid template");
            let rendered = parsed.render(&input).expect("rendered destination");
            assert!(
                parsed.matches_rendered_output(&rendered),
                "template {template} no longer recognizes its own output {rendered}"
            );
        }
    }

    #[test]
    fn render_output_is_already_url_normalized() {
        // MR #153 review (Mitri): `render()` built `rendered` via raw literal +
        // variable string concatenation and previously returned it as-is, while
        // `matches_rendered_output` normalizes its input through
        // `Url::parse(..).to_string()`. For every template in practice —
        // authored with canonical URL syntax, e.g. copied verbatim from a cloud
        // storage endpoint — the two forms already coincide, but nothing
        // enforced that. This asserts `render()`'s output is provably idempotent
        // under the same normalization `matches_rendered_output` applies to its
        // input, i.e. both sides now demonstrably go through the same path
        // rather than merely happening to agree.
        //
        // Residual scope note: this does not normalize the template's *literal*
        // segments themselves, only `render()`'s final output. A template
        // literal authored with non-canonical URL syntax (mixed-case host, a
        // redundant default port) would still produce output that fails to
        // match its own template shape in `matches_rendered_output`, which
        // compares against those raw literals. Closing that fully would mean
        // normalizing the template's literal prefix at `parse()` time — a
        // larger change, deferred as low-priority: real templates are operator
        // config copied from canonical storage endpoint URLs, not hand-authored
        // with unusual host casing or explicit default ports.
        let template = ThumbnailPathTemplate::parse(
            "https://thumbs.example/{resource_address_md5}/{resource_filename}.auto.{output_encoding}",
        )
        .expect("valid template");

        let rendered = template
            .render(&context(
                "https://source.example/path/file.jpg",
                "https://source.example/path/file.jpg?version=1",
            ))
            .expect("rendered destination");

        let reparsed = reqwest::Url::parse(&rendered)
            .expect("valid URL")
            .to_string();
        assert_eq!(
            rendered, reparsed,
            "render() output must already be in the URL-normalized form"
        );
        assert!(template.matches_rendered_output(&rendered));
    }

    #[test]
    fn from_name_recognizes_every_var_kind_variant_and_rejects_unknown_names() {
        // MR #153 review (Artem): is `VarKind::from_name` tested for all
        // variants/edge cases? This exhaustive match (no `_` arm) makes adding a
        // new `VarKind` variant without also updating this test a compile error,
        // the same pattern used by `output_encodings_recognize_every_encoding_format_variant`.
        fn canonical_name(kind: VarKind) -> &'static str {
            match kind {
                VarKind::ResourcePath => "resource_path",
                VarKind::ResourceFilename => "resource_filename",
                VarKind::ResourceAddressMd5 => "resource_address_md5",
                VarKind::ResourceIdentityMd5 => "resource_identity_md5",
                VarKind::Width => "width",
                VarKind::Height => "height",
                VarKind::OutputEncoding => "output_encoding",
                VarKind::Quality => "quality",
            }
        }

        for kind in [
            VarKind::ResourcePath,
            VarKind::ResourceFilename,
            VarKind::ResourceAddressMd5,
            VarKind::ResourceIdentityMd5,
            VarKind::Width,
            VarKind::Height,
            VarKind::OutputEncoding,
            VarKind::Quality,
        ] {
            let name = canonical_name(kind);
            assert_eq!(
                VarKind::from_name(name),
                Some(kind),
                "from_name does not recognize the canonical name for {kind:?}"
            );
        }

        for unknown in [
            "Width",
            "WIDTH",
            " width",
            "width ",
            "resource_adress_md5",
            "",
            "unknown_var",
        ] {
            assert_eq!(
                VarKind::from_name(unknown),
                None,
                "from_name should reject {unknown:?}"
            );
        }
    }

    #[test]
    fn rejects_malformed_braces() {
        for malformed in [
            "https://thumbs.example/{resource_address_md5",
            "https://thumbs.example/resource_address_md5}",
            "https://thumbs.example/{{resource_address_md5}}",
            "https://thumbs.example/{}",
        ] {
            let error = ThumbnailPathTemplate::parse(malformed)
                .expect_err("malformed template must be rejected");
            assert!(error.to_string().contains("brace"), "{error}");
        }
    }

    #[test]
    fn rejects_unknown_variable_and_names_it() {
        let error =
            ThumbnailPathTemplate::parse("https://thumbs.example/{resource_adress_md5}/thumb.png")
                .expect_err("unknown variable must be rejected");

        assert!(error.to_string().contains("resource_adress_md5"), "{error}");
    }

    #[test]
    fn rejects_template_that_cannot_distinguish_sources() {
        let error = ThumbnailPathTemplate::parse(
            "https://thumbs.example/{width}x{height}/thumbnail.{output_encoding}",
        )
        .expect_err("collision-prone template must be rejected");

        assert!(error.to_string().contains("source"), "{error}");
    }

    #[test]
    fn rejects_non_absolute_rendered_destination() {
        let error = ThumbnailPathTemplate::parse(
            "relative/{resource_address_md5}/thumbnail.{output_encoding}",
        )
        .expect_err("relative destination must be rejected");

        assert!(error.to_string().contains("absolute"), "{error}");
    }

    #[test]
    fn omitting_identity_hash_overwrites_new_source_versions() {
        let template = ThumbnailPathTemplate::parse(
            "https://thumbs.example/{resource_address_md5}/{width}x{height}.png",
        )
        .expect("valid address-keyed template");

        let version_one = template
            .render(&context(
                "https://source.example/image.jpg",
                "https://source.example/image.jpg?version=1",
            ))
            .expect("version one destination");
        let version_two = template
            .render(&context(
                "https://source.example/image.jpg",
                "https://source.example/image.jpg?version=2",
            ))
            .expect("version two destination");

        assert_eq!(version_one, version_two);
    }

    #[test]
    fn identity_hash_produces_version_specific_destinations() {
        let template = ThumbnailPathTemplate::parse(
            "https://thumbs.example/{resource_address_md5}/{resource_identity_md5}.png",
        )
        .expect("valid version-keyed template");

        let version_one = template
            .render(&context(
                "https://source.example/image.jpg",
                "https://source.example/image.jpg?version=1",
            ))
            .expect("version one destination");
        let version_two = template
            .render(&context(
                "https://source.example/image.jpg",
                "https://source.example/image.jpg?version=2",
            ))
            .expect("version two destination");

        assert_ne!(version_one, version_two);
    }

    #[derive(Parser, Debug)]
    struct TemplateCli {
        #[command(flatten)]
        thumbnail_path: crate::ThumbnailPathArgs,
    }

    #[test]
    fn runtime_config_requires_thumbnail_path_template() {
        let command = TemplateCli::command();
        let template_arg = command
            .get_arguments()
            .find(|argument| argument.get_id() == "thumbnail-path-template")
            .expect("thumbnail path template clap argument");

        assert!(template_arg.is_required_set());
    }

    #[test]
    fn runtime_config_rejects_invalid_template_with_actionable_error() {
        let error = TemplateCli::try_parse_from([
            "thumbnail-service",
            "--thumbnail-path-template",
            "relative/{unknown}",
        ])
        .expect_err("invalid template must fail startup parsing");

        assert!(error.to_string().contains("unknown"));
    }

    #[test]
    fn matches_rendered_output_for_centralized_dev_template() {
        let template = ThumbnailPathTemplate::parse(
            "https://examplestorage.blob.core.windows.net/example/thumbnails/\
             {resource_address_md5}/{resource_identity_md5}/{width}x{height}/q{quality}/\
             {resource_filename}.auto.{output_encoding}",
        )
        .expect("valid dev template");
        let rendered = template
            .render(&context(
                "https://examplestorage.blob.core.windows.net/example/user/path-patterns/20220704_112348.jpg",
                "https://examplestorage.blob.core.windows.net/example/user/path-patterns/20220704_112348.jpg?versionid=1",
            ))
            .expect("rendered destination");

        assert!(template.matches_rendered_output(&rendered));
        assert!(template.matches_rendered_output(&format!("{rendered}?versionid=1")));
        assert!(!template.matches_rendered_output(
            "https://examplestorage.blob.core.windows.net/example/user/path-patterns/20220704_112348.jpg"
        ));
        assert!(!template.matches_rendered_output(
            "https://examplestorage.blob.core.windows.net/example/thumbnails/ordinary-source.png"
        ));
    }

    #[test]
    fn matches_rendered_output_for_colocated_template() {
        let template = ThumbnailPathTemplate::parse(
            "{resource_path}/.thumbs/{width}x{height}/{resource_filename}.auto.{output_encoding}",
        )
        .expect("valid colocated template");
        let rendered = template
            .render(&ThumbnailPathContext {
                height: 256,
                ..context(
                    "https://foo.example/path/file.jpg",
                    "https://foo.example/path/file.jpg?version=1",
                )
            })
            .expect("rendered destination");

        assert!(template.matches_rendered_output(&rendered));
        assert!(!template.matches_rendered_output("https://foo.example/path/file.jpg"));
        assert!(!template
            .matches_rendered_output("https://storage.example/thumbnails/ordinary-source.png"));
    }

    #[test]
    fn matches_rendered_output_for_source_relative_template() {
        // Per-bucket/source-relative layout from design.md: a sibling `_thumbnails`
        // directory next to the source, distinct from the Nucleus-compatible
        // colocated `.thumbs` shape covered above.
        let template = ThumbnailPathTemplate::parse(
            "{resource_path}/_thumbnails/{width}x{height}/{resource_filename}.{output_encoding}",
        )
        .expect("valid source-relative template");
        let rendered = template
            .render(&context(
                "https://bucket.example/assets/photos/image.jpg",
                "https://bucket.example/assets/photos/image.jpg?version=1",
            ))
            .expect("rendered destination");

        assert_eq!(
            rendered,
            "https://bucket.example/assets/photos/_thumbnails/256x128/image.jpg.png"
        );
        assert!(template.matches_rendered_output(&rendered));
        assert!(template.matches_rendered_output(&format!("{rendered}?version=1")));
        assert!(!template.matches_rendered_output("https://bucket.example/assets/photos/image.jpg"));
        assert!(!template.matches_rendered_output(
            "https://bucket.example/assets/photos/_thumbnails/ordinary-source.png"
        ));
    }

    #[test]
    fn matches_rendered_output_treats_percent_encoded_question_mark_as_literal_path_content() {
        // MR #153 review (Artem): what if there's a '?' in the filename? A raw,
        // unencoded '?' always starts a URL's query component per RFC 3986, so a
        // literal filename containing '?' must be percent-encoded (`%3F`) to be
        // valid path content at all. This pins that a percent-encoded '?' in the
        // rendered path is treated as ordinary literal text (never mistaken for
        // a query delimiter), while a real trailing query string (e.g. a
        // storage version marker appended after generation) is still stripped
        // correctly for matching.
        let template = ThumbnailPathTemplate::parse(
            "{resource_path}/.thumbs/{width}x{height}/{resource_filename}.auto.{output_encoding}",
        )
        .expect("valid colocated template");
        let rendered = template
            .render(&ThumbnailPathContext {
                height: 256,
                ..context(
                    "https://foo.example/path/what%3Fnow.jpg",
                    "https://foo.example/path/what%3Fnow.jpg?version=1",
                )
            })
            .expect("rendered destination");

        assert!(
            rendered.contains("what%3Fnow.jpg"),
            "percent-encoded '?' should survive rendering verbatim: {rendered}"
        );
        assert!(template.matches_rendered_output(&rendered));
        assert!(template.matches_rendered_output(&format!("{rendered}?version=2")));
        assert!(template.matches_rendered_output(&format!("{rendered}#fragment")));
    }

    #[test]
    fn output_encodings_recognize_every_encoding_format_variant() {
        // MR #153 review (item 1): OUTPUT_ENCODINGS in take_output_encoding is a
        // hand-maintained list, independent of the EncodingFormat enum. Nothing
        // fails today if someone adds an EncodingFormat variant without updating
        // that list — matches_rendered_output would silently stop recognizing
        // that format's own generated output, degrading recursion prevention
        // with no test failure.
        //
        // This exhaustive match (no `_` arm) makes that a compile error instead:
        // adding a new EncodingFormat variant forces this test to be updated in
        // the same change.
        fn lowercase_name(format: crate::EncodingFormat) -> String {
            match format {
                crate::EncodingFormat::Png
                | crate::EncodingFormat::Jpeg
                | crate::EncodingFormat::Gif
                | crate::EncodingFormat::Webp => format.to_string().to_lowercase(),
            }
        }

        for format in [
            crate::EncodingFormat::Png,
            crate::EncodingFormat::Jpeg,
            crate::EncodingFormat::Gif,
            crate::EncodingFormat::Webp,
        ] {
            let lowercase = lowercase_name(format);
            assert!(
                take_output_encoding(&lowercase).is_some(),
                "OUTPUT_ENCODINGS does not recognize EncodingFormat::{format:?} ('{lowercase}')"
            );
        }
    }

    #[test]
    fn matches_rendered_output_false_positive_on_shape_matching_legitimate_upload() {
        // MR #153 review (item 2): matches_rendered_output is purely structural.
        // A legitimate user upload that happens to land at a path shaped like this
        // template's output is indistinguishable from a real generated thumbnail
        // and is silently treated as "our own output" (skipped, never
        // thumbnailed). This is an inherent limitation of shape-matching, not a
        // bug — pinning it here makes it a conscious, documented boundary rather
        // than a surprise.
        let template = ThumbnailPathTemplate::parse(
            "{resource_path}/_thumbnails/{width}x{height}/{resource_filename}.{output_encoding}",
        )
        .expect("valid source-relative template");

        // A real user upload that was never generated by this service, but whose
        // path happens to match the configured template's shape exactly.
        let coincidental_upload = "https://bucket.example/assets/_thumbnails/256x256/photo.png";

        assert!(
            template.matches_rendered_output(coincidental_upload),
            "shape-matching cannot distinguish a coincidentally-shaped legitimate \
             upload from real generated output; this is documented, not asserted \
             as a bug"
        );
    }

    #[test]
    fn adjacent_numeric_variables_without_separator_are_rejected_at_parse_time() {
        // MR #153 review (item 3, previously a documented-limitation test):
        // take_digits is greedy, so adjacent numeric variables (e.g. `{width}{height}`)
        // render "256256" (or "256128") but take_digits consumes all digits for
        // {width}, leaving {height} with nothing to match.  The parse-time round-trip
        // check (task 8.4) now catches and rejects such templates at startup rather
        // than accepting them and silently breaking the recursion guard.
        // Real templates always separate width and height with a literal (e.g. "x"):
        // `{width}x{height}` works because the literal "x" is unambiguous.
        let err = ThumbnailPathTemplate::parse(
            "https://thumbs.example/{resource_address_md5}/{width}{height}/thumb.png",
        )
        .expect_err("adjacent numeric variables must be rejected at parse time");

        assert!(
            matches!(err, ThumbnailPathError::FailsRoundTrip(_)),
            "expected FailsRoundTrip, got: {err}"
        );
    }

    // 8.5 — round-trip property test for every template documented in
    // README, values.yaml, and .env.example.  parse() now enforces this via the
    // smoke-render check, so these all pass via that mechanism; this test names
    // the requirement and provides a labeled anchor for a CI failure.
    #[test]
    fn documented_templates_all_round_trip() {
        let documented_templates = [
            // values.yaml example 1 — centralized, content-hash keyed layout
            "https://examplestorage.blob.core.windows.net/example/thumbnails/\
             {resource_address_md5}/{resource_identity_md5}/{width}x{height}/q{quality}/\
             {resource_filename}.auto.{output_encoding}",
            // values.yaml example 2 — source-relative sibling `_thumbnails` directory
            "{resource_path}/_thumbnails/{width}x{height}/{resource_filename}.{output_encoding}",
            // .env.example — centralized without identity hash
            "https://your-storage-account.blob.core.windows.net/your-container/thumbnails/\
             {resource_address_md5}/{width}x{height}/{resource_filename}.auto.{output_encoding}",
            // README / tests — colocated sibling `.thumbs` directory
            "{resource_path}/.thumbs/{width}x{height}/{resource_filename}.auto.{output_encoding}",
        ];

        let smoke = ThumbnailPathContext {
            resource_address: "https://storage.example/path/source.png",
            resource_identity: "https://storage.example/path/source.png?version=1",
            width: 256,
            height: 256,
            output_encoding: "PNG",
            quality: 85,
        };

        for template_str in &documented_templates {
            let template = ThumbnailPathTemplate::parse(*template_str).unwrap_or_else(|e| {
                panic!("documented template must parse: {e}\n  template: {template_str}")
            });
            let rendered = template.render(&smoke).unwrap_or_else(|e| {
                panic!("documented template must render: {e}\n  template: {template_str}")
            });
            assert!(
                template.matches_rendered_output(&rendered),
                "documented template does not recognize its own rendered output:\n  \
                 template: {template_str}\n  rendered: {rendered}"
            );
        }
    }

    #[test]
    fn runtime_config_parses_template_into_typed_value() {
        let cli = TemplateCli::try_parse_from([
            "thumbnail-service",
            "--thumbnail-path-template",
            "{resource_path}/.thumbs/{width}x{height}/{resource_filename}.{output_encoding}",
        ])
        .expect("valid template should parse");

        assert_eq!(
            cli.thumbnail_path
                .thumbnail_path_template
                .render(&context(
                    "https://source.example/path/image.jpg",
                    "https://source.example/path/image.jpg?version=1",
                ))
                .expect("rendered destination"),
            "https://source.example/path/.thumbs/256x128/image.jpg.png"
        );
    }
}
