# Shared libraries (`common/`) — component map

> **Applies to:** `common/observability` (crates `otel-tools`, `tonic-observability`, `axum-observability`, `otel-asserts`, `protos`, `testing`) and `common/url-normalizer` (crate `url_normalizer`) · **Released:** not in the released NGC inventory (libraries, consumed at build time) · **This repo's snapshot:** observability **0.14.0** + `[Unreleased]` ([`Cargo.toml`](../../../../common/observability/Cargo.toml) `[workspace.package] version`, [`CHANGELOG.md`](../../../../common/observability/CHANGELOG.md); release commit "Release observability 0.0.0-manual.06dc4fad", which changed no library code: test helpers only, crate metadata, and the README's internal release and toolchain-bump procedures removed), MSRV 1.91.0, edition 2024; url-normalizer **0.1.0** ([`Cargo.toml`](../../../../common/url-normalizer/Cargo.toml)) · **Sources:** [`common/observability/README.md`](../../../../common/observability/README.md), [`common/observability/docs/`](../../../../common/observability/docs/), [`common/url-normalizer/README.md`](../../../../common/url-normalizer/README.md), consumer `Cargo.toml` files

Neither library has an upstream component skill in the [source skill package](../../NOTICE.md); [`SOURCE_VERSIONS.md`](../../../../SOURCE_VERSIONS.md) records their source revisions. Both are Rust-only and are used through path dependencies, either on `common/` or on a copy vendored into the consumer; nothing here is published to a registry from this repo (the observability crates' `publish` key names an internal registry, which does not apply outside NVIDIA).

## Who uses what

| Consumer | Uses |
|---|---|
| Envoy auth extension ([`Cargo.toml`](../../../../permissions/reference/envoy-auth-extension/Cargo.toml)) | `otel-tools`, `tonic-observability`, vendored copies in `vendor/` |
| Permission Service ([`crates/permission-service/Cargo.toml`](../../../../permissions/reference/permission-service/crates/permission-service/Cargo.toml)) | `axum-observability`, path to `common/observability` |
| Thumbnail Service ([`Cargo.toml`](../../../../thumbnails/reference/thumbnail-service/Cargo.toml), [`crates/service/Cargo.toml`](../../../../thumbnails/reference/thumbnail-service/crates/service/Cargo.toml)) | `otel-tools`, `tonic-observability` (workspace), `axum-observability` (service crate), vendored copies in `vendor/` |
| Storage Service ([`Cargo.toml`](../../../../storage/reference/storage-service/Cargo.toml)) | `url_normalizer`, only in its integration tests (`tests/url_normalizer_v1alpha.rs`, `_v1beta.rs`) |

The Storage Service's request-path URL normalization (`config.urlNormalizer.enabled`) is its own layer, [`storage-service/src/backend/url_normalizer/`](../../../../storage/reference/storage-service/storage-service/src/backend/url_normalizer/), documented in [`docs/helm/url-normalizer.rst`](../../../../storage/reference/storage-service/docs/helm/url-normalizer.rst). It is **not** the `common/url-normalizer` crate and follows different rules (path-only; collapses empty segments and resolves `.`/`..` after percent-decoding `%2F` and `%2E`).

## observability

| Crate | Purpose | Detail |
|---|---|---|
| `otel-tools` | OTLP exporter setup bound to `tracing`; configured only by environment variables; providers shut down when guards drop (call the shutdown handle on abort paths) | [`README.md`](../../../../common/observability/README.md#otel-tools) |
| `tonic-observability` | Trace and metric layers for tonic gRPC clients and servers; W3C trace-context propagation; streaming-aware; not configurable | [server metrics A66](../../../../common/observability/docs/layer/grpc-server-metrics-a66.md), [server spans](../../../../common/observability/docs/layer/grpc-server-spans.md), [client metrics A66](../../../../common/observability/docs/layer/grpc-client-metrics-a66.md), [client spans](../../../../common/observability/docs/layer/grpc-client-spans.md), [layers overview](../../../../common/observability/docs/layer/grpc-layers.md) |
| `axum-observability` | Trace and metric layers for axum HTTP servers | [server metrics](../../../../common/observability/docs/layer/http-server-metrics.md), [server spans](../../../../common/observability/docs/layer/http-server-spans.md), [protocol](../../../../common/observability/docs/layer/http-server-protocol.md) |
| `otel-asserts`, `testing` | Test helpers | — |
| `protos` | Test protos | — |

Environment variables read by `otel-tools` (from its source): the standard `OTEL_SDK_DISABLED`, `OTEL_SERVICE_NAME`, `OTEL_RESOURCE_ATTRIBUTES`, `OTEL_PROPAGATORS`, `OTEL_TRACES_EXPORTER`, `OTEL_METRICS_EXPORTER`, `OTEL_LOGS_EXPORTER`, `OTEL_EXPORTER_OTLP_{ENDPOINT,PROTOCOL,HEADERS,TIMEOUT,COMPRESSION}` and per-signal variants, plus `OTEL_RUST_OTLP_TRACES_FILTER` / `OTEL_RUST_OTLP_LOGS_FILTER`. Put OTLP auth headers in a Secret-sourced env var, not in chart values.

Version table (Tonic, OTel SDK, Axum per release) is in the [README](../../../../common/observability/README.md#releases). `[Unreleased]` removes the deprecated OTel gRPC server metric layer (use A66) and changes A66 message sizes to exclude gRPC framing; dashboards built on the old layer or on framed sizes change when consumers pick this up. Specifications the layers implement are under [`docs/spec/`](../../../../common/observability/docs/spec/). Background and FAQ: [`BACKGROUND.md`](../../../../common/observability/docs/BACKGROUND.md), [`FAQ.md`](../../../../common/observability/docs/FAQ.md).

Developing it: `cargo nextest run` (required runner; `cargo test --doc` for doctests), `cargo clippy --workspace --all-targets --all-features`; toolchain in [`rust-toolchain.toml`](../../../../common/observability/rust-toolchain.toml); needs `protoc`. The README warns there is no support outside NVIDIA projects and no stability guarantee.

How services expose these signals and what to monitor: [operate/monitoring.md](../operate/monitoring.md).

## url-normalizer

`url_normalizer::normalize_url(url) -> Result<String, url::ParseError>` ([`src/lib.rs`](../../../../common/url-normalizer/src/lib.rs)) parses an absolute URL, drops the fragment, sorts query parameters by key, collapses consecutive slashes in the path, removes default ports (80, 443, 21) and percent-encodes path and query. A binary of the same name wraps it (`cargo run -- "<url>"` from `common/url-normalizer`); example in [`examples/basic_usage.rs`](../../../../common/url-normalizer/examples/basic_usage.rs).

Defects:

- [`README.md`](../../../../common/url-normalizer/README.md) says "MIT License"; [`LICENSE`](../../../../common/url-normalizer/LICENSE) and `Cargo.toml` are Apache-2.0.
- The README's install snippet `url_normalizer = "0.1.0"` implies a crates.io release; consumers here use `path = "../../../common/url-normalizer"`.
- A URL without a host normalizes with the literal host `None` (`host_str().unwrap_or("None")`), rather than failing.

Related: [develop/add-your-service.md](../develop/add-your-service.md#observability), [storage-service.md](storage-service.md).
