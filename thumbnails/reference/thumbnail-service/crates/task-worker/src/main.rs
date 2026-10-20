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

//! Binary entrypoint. Everything of substance lives in the library beside it,
//! so the worker's internals are reachable from unit tests.

use thumbnail_task_worker::*;

use std::sync::Arc;

use clap::Parser;
use thumbnail_common::init_tracing;
use thumbnail_common::notifications::provisioning::{
    build_generate_filter_groups, ensure_durable_queue, EnsureQueueOutcome,
};
use thumbnail_common::outbound::{OutboundClients, OutboundClientsConfig};
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::watch;
use tracing::{error, info};

use crate::config::{ProbeCli, WorkerCli};
use crate::consumer::Consumer;
use crate::health::HealthState;
use crate::metrics::WorkerMetrics;
use crate::pipeline::Pipeline;

/// Typed errors for the top-level supervisor `select!` loop.
///
/// Each variant captures why the supervisor is terminating so control flow
/// stays typed and the `Box<dyn Error>` returned from `main` still carries a
/// meaningful message via `Display`.
#[derive(Debug)]
enum SupervisorExitError {
    ConsumerExitedWithoutShutdownSignal,
    ConsumerPanicked(String),
    ConsumerFailed(String),
    ProbeExitedWithoutShutdownSignal,
    ProbeFailed(String),
    ProbePanicked(String),
}

impl std::fmt::Display for SupervisorExitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConsumerExitedWithoutShutdownSignal => {
                write!(f, "consumer task exited without a shutdown signal")
            }
            Self::ConsumerPanicked(msg) => write!(f, "consumer task panicked: {msg}"),
            Self::ConsumerFailed(msg) => write!(f, "consumer task failed: {msg}"),
            Self::ProbeExitedWithoutShutdownSignal => {
                write!(f, "probe task exited without a shutdown signal")
            }
            Self::ProbeFailed(msg) => write!(f, "probe task failed: {msg}"),
            Self::ProbePanicked(msg) => write!(f, "probe task panicked: {msg}"),
        }
    }
}

impl std::error::Error for SupervisorExitError {}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _ = dotenvy::dotenv();

    // Checked before init_tracing so that probe subprocesses — which the
    // kubelet invokes on every readiness poll — skip OTLP initialisation
    // entirely.  A probe only needs a single HTTP round-trip; paying the cost
    // of setting up exporters and a meter provider on every probe call is
    // unnecessary overhead.
    //
    // The port still comes through clap rather than a direct `env::var`, so the
    // probe and the server it probes resolve it from one definition. Reading the
    // variable directly here would be a second configuration path, and the two
    // would disagree the moment anyone passed `--health-port` on the command
    // line — reporting a healthy worker as unhealthy.
    if std::env::args().any(|arg| arg == "--probe-ready") {
        let probe = ProbeCli::parse_from(std::env::args().filter(|arg| arg != "--probe-ready"));
        return match health::probe_ready(probe.health_port).await {
            Ok(()) => Ok(()),
            Err(reason) => Err(reason.into()),
        };
    }

    let _otel_guards = init_tracing("thumbnail-task-worker");

    let cli = WorkerCli::parse();
    let args = cli.worker.clone();
    let provisioning = args.resolve_queue_provisioning()?;

    info!(
        queue_id = %provisioning.queue_id,
        max_concurrent_conversions = args.max_concurrent_conversions,
        recycle_interval_secs = args.redelivery_recycle_interval.as_secs(),
        max_source_bytes = args.max_source_bytes,
        max_decoded_pixels = args.max_decoded_pixels,
        max_transform_peak_bytes = args.max_transform_peak_bytes,
        "Starting thumbnail task-worker"
    );

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let health = Arc::new(HealthState::default());
    let metrics = Arc::new(WorkerMetrics::new(opentelemetry::global::meter(
        "thumbnail-task-worker",
    )));

    // Before outbound auth build and provisioning, not after. `OutboundClients::build`
    // waits up to TOKEN_READY_TIMEOUT (60s) for the first bearer, and provisioning
    // waits on the credential sidecar with retries at 10s, 20s, 40s — routinely over
    // a minute on a cold start. Until something is listening, liveness begins probing
    // ten seconds in, fails its three attempts against a closed port, and the kubelet
    // SIGKILLs a worker whose only problem is that it is still starting. Every cold
    // start in dev cost a restart and about two minutes that way; the token gate would
    // have the same fate (its 60s timeout never fires against a ~50s kill budget).
    //
    // Serving early is not a weaker check, because the two probes mean different
    // things: `/healthz` answers "this process is alive", which is true here, and
    // `/readyz` stays false until the consume stream exists, so nothing routes
    // work to a worker that is not consuming yet. Binding still happens on the
    // startup path so a port already in use is a loud startup failure rather than
    // something discovered later through probe timeouts. A startupProbe in the
    // chart covers the remaining window until `/healthz` answers.
    let probe_listener = health::bind(args.health_port).await?;
    let mut probe = tokio::spawn(health::serve(
        probe_listener,
        health.clone(),
        metrics.clone(),
        shutdown_rx.clone(),
    ));

    let outbound = Arc::new(
        OutboundClients::build(OutboundClientsConfig {
            auth: cli.auth,
            storage_endpoint: args.storage_grpc_endpoint.clone(),
            // The worker publishes nothing: association events are the service's
            // to emit, after its own write succeeds.
            event_publishing_endpoint: None,
            event_consumer_endpoint: Some(args.notifications_event_consumer_endpoint.clone()),
        })
        .await?,
    );

    let consumer_channel = outbound
        .event_consumer_channel
        .clone()
        .ok_or("event-consumer channel was not built despite a configured endpoint")?;
    let thumbnail_service = connect_lazy(&args.thumbnail_service_grpc_endpoint)?;

    // Provision WorkerEventQueue before consuming, the same fail-fast contract the server
    // applies to its own queue: a worker that starts without its queue consumes
    // nothing while looking healthy.
    let mut provision_shutdown = shutdown_rx.clone();
    // Kept so the consume loop can re-provision with exactly these groups after a
    // NOT_FOUND, rather than recreating the queue with a different subscription.
    let filter_groups = build_generate_filter_groups(&provisioning.resource_prefixes);
    match ensure_durable_queue(
        consumer_channel.clone(),
        outbound.interceptor.clone(),
        outbound.refresher.clone(),
        &provisioning.queue_id,
        filter_groups.clone(),
        &mut provision_shutdown,
        &args.stream_runtime_args(),
    )
    .await
    {
        Ok(EnsureQueueOutcome::Created) => {
            info!(queue_id = %provisioning.queue_id, "Durable queue created")
        }
        Ok(EnsureQueueOutcome::AlreadyExisted) => {
            info!(queue_id = %provisioning.queue_id, "Durable queue already existed")
        }
        Ok(EnsureQueueOutcome::ShutdownAborted) => return Ok(()),
        Err(e) => return Err(e.to_string().into()),
    }

    let consumer = Consumer {
        args: args.clone(),
        queue_id: provisioning.queue_id.clone(),
        filter_groups,
        consumer_channel,
        interceptor: outbound.interceptor.clone(),
        refresher: outbound.refresher.clone(),
        token_gate: outbound.token_gate.clone(),
        pipeline: Arc::new(Pipeline {
            args: args.clone(),
            outbound: outbound.clone(),
            thumbnail_service,
            metrics: metrics.clone(),
        }),
        metrics,
        health: health.clone(),
    };

    let mut consuming = tokio::spawn({
        let shutdown_rx = shutdown_rx.clone();
        async move { consumer.run(shutdown_rx).await }
    });

    // Supervise the three concurrent critical tasks: signal, consumer, and probe.
    // If either background task exits before a shutdown signal arrives the
    // process cannot continue in a useful state.  The consumer exit path marks
    // health failed before exiting so Kubernetes observes a failing liveness
    // probe and schedules a restart; the nonzero exit also satisfies the pod's
    // restart policy directly.
    tokio::select! {
        () = await_termination() => {
            info!("Shutdown signal received; stopping intake and draining in-flight conversions");
            health.draining();
            let _ = shutdown_tx.send(true);

            if let Err(e) = consuming.await {
                error!(error = %e, "Consumer task ended abnormally");
            }
            probe.abort();
        }

        result = &mut consuming => {
            let reason = match result {
                Ok(_) => SupervisorExitError::ConsumerExitedWithoutShutdownSignal,
                Err(ref e) if e.is_panic() => SupervisorExitError::ConsumerPanicked(e.to_string()),
                Err(ref e) => SupervisorExitError::ConsumerFailed(e.to_string()),
            };
            error!(error = %reason);
            health.draining();
            let _ = shutdown_tx.send(true);
            probe.abort();
            return Err(reason.into());
        }

        result = &mut probe => {
            let reason = match result {
                Ok(Ok(())) => SupervisorExitError::ProbeExitedWithoutShutdownSignal,
                Ok(Err(ref e)) => SupervisorExitError::ProbeFailed(e.to_string()),
                Err(ref e) => SupervisorExitError::ProbePanicked(e.to_string()),
            };
            error!(error = %reason);
            let _ = shutdown_tx.send(true);
            consuming.abort();
            return Err(reason.into());
        }
    }

    info!("task-worker stopped");
    Ok(())
}

/// The subset of stream tuning `ensure_durable_queue` reads. The worker keeps its
/// own flat settings rather than the server's full `RuntimeConfigArgs`, which
/// carries a great deal it has no business reading.
///
/// From `Default` rather than a parse of an empty argv: clap treats the
/// environment as a value source, so parsing would quietly adopt any `STREAM_*`
/// or `EVENT_PROCESSING_*` the pod exports for the server, in a struct where the
/// worker only means to set three fields.
/// Build the thumbnail-service channel without dialling it.
///
/// Lazily, and deliberately. The worker does not need thumbnail-service until it
/// has a converted thumbnail to associate, which is necessarily after it has
/// consumed work — so refusing to start because that service is not up yet
/// buys nothing and costs availability.
///
/// It also breaks a real deadlock. WorkerEventQueue must be consumed before anything
/// publishes generate events, so thumbnail-service waits for the worker to be
/// ready; if the worker in turn waited for thumbnail-service, neither would ever
/// start. Connecting on first use resolves the name when it matters instead of
/// at the least convenient moment.
fn connect_lazy(
    endpoint: &str,
) -> Result<tonic::transport::Channel, Box<dyn std::error::Error + Send + Sync>> {
    let url = if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        endpoint.to_string()
    } else {
        format!("http://{endpoint}")
    };
    Ok(tonic::transport::Endpoint::from_shared(url)?
        .connect_timeout(std::time::Duration::from_secs(10))
        .connect_lazy())
}

async fn await_termination() {
    let mut terminate =
        signal(SignalKind::terminate()).expect("SIGTERM handler must be installable");
    let mut interrupt =
        signal(SignalKind::interrupt()).expect("SIGINT handler must be installable");

    tokio::select! {
        _ = terminate.recv() => {}
        _ = interrupt.recv() => {}
    }
}
