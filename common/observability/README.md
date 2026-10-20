# tonic-observability

> **WARNING**: We are a small team and do not provide support for `tonic-observability` outside of our projects. We make no stability guarantees, and may make breaking changes or aggressively bump the MSRV or any dependencies. We may also reject any new features. `tonic-observability` is currently work-in-progress.

## Overview

### MSRV

The minimum supported Rust version is the version released at least 3 months ago (unless a security issue requires a more recent version).

### otel-tools

`otel-tools` is a lower-level library that provides convenience functions to setup OTLP exporters and integrate OTLP exporters with Rust's `tracing` framework.

#### Exporter settings

Exporters can only be configured via environment variables. It's recommended to use something like [dotenv](https://www.dotenv.org/) to provide settings for running service locally. (It isn't possible to prevent the Rust OpenTelemetry SDK from using environment variables anyway.)

#### Provider/exporter shut down

Providers are shut down when their guards are dropped. This is convenient for most programs and guarantees data is exported before the program exits.

Programs that abort instead of unwinding will never drop the guards. This could be because of the panic strategy, because a function like [`std::process::abort`](https://doc.rust-lang.org/std/process/fn.abort.html) or [`std::process::exit`](https://doc.rust-lang.org/std/process/fn.exit.html) is called, or other reasons. In this case, the provider *must* be shut down manually to guarantee the data is exported. (For example, in a panic handler.) See the documentation of the guards on how to create a shutdown handle to manually shut down the providers.

### tonic-observability

`tonic-observability` is an opinionated library to provide trace and metrics layers for [`tonic`](https://crates.io/crates/tonic)-based Rust gRPC clients and servers.

* Instrument `tonic`-based gRPC clients and servers automatically.
* Integrates with `tracing` and `opentelemetry`.
* Propagate W3C Trace Context headers automatically, with no extra configuration.
* Handles streaming gRPC calls.
* No C++ code.

The layers cannot be customized/configured. If you need to customize a layer, you can use [`tower_http::trace::TraceLayer`](https://docs.rs/tower-http/0.6.1/tower_http/trace/index.html) instead. This library does not help setting up exporting of traces or metrics.

Each layer has detailed documentation on the specifications/semantic conventions it implements:

* [Server metric layer (A66)](./docs/layer/grpc-server-metrics-a66.md)
* [Server trace layer](./docs/layer/grpc-server-spans.md)
* [Client metric layer (A66)](./docs/layer/grpc-client-metrics-a66.md)
* [Client trace layer](./docs/layer/grpc-client-spans.md)
* Server metric layer (OTel): Omniverse has decided to move away from OTel gRPC metrics; this layer will be deprecated soon.

#### Metrics return values

Avoid returning `Err(Status::ok())` from the service/server. This is an unusual pattern that is unsupported by `tonic`, and will result in incorrect metrics also.

### axum-observability

`axum-observability` is an opinionated library to provide trace and metrics layers for [`axum`](https://crates.io/crates/axum)-based Rust HTTP servers. Technically, the layers will work with any `tower`-based service, however only `axum` is tested. `axum` is good for micro-services, and is backed by `tokio`.

Each layer has detailed documentation on the specifications/semantic conventions it implements:

* [Server metric layer](./docs/layer/http-server-metrics.md)
* [Server trace layer](./docs/layer/http-server-spans.md)

## Releases

| Version     | Date       | MSRV   | Tonic     | Otel SDK  | Axum      |
|-------------|------------|--------|-----------| ----------|-----------|
| `0.14.0`    | 2026-05-01 | 1.91.0 | `v0.14.*` | `v0.31.*` | `v0.8.*`  |
| `0.13.0`    | 2026-04-03 | 1.91.0 | `v0.14.*` | `v0.31.*` | `v0.8.*`  |
| `0.12.0`    | 2026-02-18 | 1.91.0 | `v0.14.*` | `v0.31.*` | `v0.8.*`  |
| `0.11.0`    | 2025-11-21 | 1.91.0 | `v0.14.*` | `v0.31.*` | N/A       |
| `0.10.0`    | 2025-11-03 | 1.90.0 | `v0.14.*` | `v0.31.*` | N/A       |
| `0.9.0`     | 2025-09-18 | 1.90.0 | `v0.13.*` | `v0.30.*` | N/A       |
| `0.8.0`     | 2025-08-08 | 1.89.0 | `v0.13.*` | `v0.30.*` | N/A       |
| `0.7.0`     | 2025-07-22 | 1.88.0 | `v0.13.*` | `v0.30.*` | N/A       |
| `0.6.1`     | 2025-07-10 | 1.88.0 | `v0.13.*` | `v0.30.*` | N/A       |
| `0.6.0`     | 2025-06-30 | 1.88.0 | `v0.13.*` | `v0.30.*` | N/A       |
| `0.5.0`     | 2025-06-30 | 1.86.0 | `v0.13.*` | `v0.30.*` | N/A       |
| `0.4.0`     | 2025-06-30 | 1.86.0 | `v0.13.*` | `v0.29.*` | N/A       |
| `0.3.0`     | 2025-04-11 | 1.85.1 | `v0.13.*` | `v0.29.*` | N/A       |
| `0.2.0`     | 2025-03-25 | 1.85.0 | `v0.12.*` | `v0.29.*` | N/A       |
| `0.1.1`     | 2025-03-13 | 1.84.0 | `v0.12.*` | `v0.27.*` | N/A       |
| `0.1.0-rc4` | 2025-03-03 | 1.84.0 | `v0.12.*` | `v0.27.*` | N/A       |
| `0.1.0-rc3` | 2025-02-12 | 1.84.0 | `v0.12.*` | `v0.27.*` | N/A       |
| `0.1.0-rc2` | 2025-02-10 | 1.84.0 | `v0.12.*` | `v0.27.*` | N/A       |
| `0.1.0-rc1` | 2025-02-04 | 1.84.0 | `v0.12.*` | `v0.27.*` | N/A       |

## FAQ

See [FAQ](docs/FAQ.md).

## Background

See [background](docs/BACKGROUND.md).

## Developer notes

### Tooling

- [Rust/Cargo](https://doc.rust-lang.org/book/ch01-01-installation.html)
- [`protoc`](https://github.com/protocolbuffers/protobuf)
- [`cargo-nextest`](https://nexte.st/docs/installation/from-source/) - required test runner
  * `cargo install --locked cargo-nextest`
- [`grcov`](https://github.com/mozilla/grcov) - for code coverage
  * `cargo install --locked grcov`

Unit tests must be run with `cargo nextest`, and not `cargo test`. However, currently [`cargo nextest` can't run doctests](https://github.com/nextest-rs/nextest/issues/16).

### Commands

```bash
# check source code is valid
cargo check --workspace
# apply auto-formatting
cargo fmt --all
# linting
cargo clippy --workspace --all-targets --all-features
# run all tests (except doctests)
cargo nextest run
# run all doctests
cargo test --doc
# run and serve code coverage
./cov.sh
# build the documentation for the local package and all dependencies
cargo doc --open
```
