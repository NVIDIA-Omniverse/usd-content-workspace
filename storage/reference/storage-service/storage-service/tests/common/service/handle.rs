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

use std::{
    io::{stderr, BufRead, BufReader, Write},
    mem::ManuallyDrop,
    process::{Command, Stdio},
    sync::mpsc::{self, Sender},
    thread::JoinHandle,
    time::Duration,
};

use regex::Regex;
use storage_core_util::ci::read_ports;
use tracing::info;

use crate::common::{env::get_logs_folder_path, process::DeathSignalExt};

lazy_static::lazy_static! {
    static ref METRICS_PORT_REGEX: Regex = Regex::new(r#"prometheus server is listening on 0\.0\.0\.0:(\d+)"#).unwrap();
}

/// Storage service process handle.
#[derive(Debug)]
pub struct ServiceHandle {
    grpc_port: u16,
    rest_port: u16,
    metrics_port: u16,
    launch_thread: ManuallyDrop<JoinHandle<()>>,
    terminate_tx: Sender<()>,
}

impl ServiceHandle {
    /// Spawn a storage service process with the specified parameters.
    pub fn spawn(
        exe: &'static str,
        args: impl IntoIterator<Item = impl Into<String>>,
        env: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        // Create a channel to receive ports from the launch thread
        let (tx, rx) = mpsc::channel();
        let (prometheus_port_tx, prometheus_port_rx) = mpsc::channel();
        let (terminate_tx, terminate_rx) = mpsc::channel();

        // This is a little jank, but the deathsignal code use PR_SET_PDEATHSIG, which will send the
        // death signal to the child when the parent *thread* exits, so we need to spawn the subprocess
        // from a thread that lives as long as the process
        let logs_path = get_logs_folder_path();
        let args: Vec<String> = args.into_iter().map(Into::into).collect();

        let env = get_process_env(env);
        let launch_thread = std::thread::spawn(move || {
            let mut command = Command::new(exe);
            command.death_signal();

            command.args(args);
            command.envs(env);
            command.stdout(Stdio::piped()).stderr(Stdio::piped());

            info!(?command, "Launching a storage service process");
            let mut child = command.spawn().unwrap();

            let pid = child.id();
            let mut stdout_file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(logs_path.join(format!("storage-service.{:08}.stdout.log", pid)))
                .unwrap();

            let mut stderr_file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(logs_path.join(format!("storage-service.{:08}.stderr.log", pid)))
                .unwrap();

            // Capture and log subprocess output at debug level
            let child_stdout = child.stdout.take().unwrap();
            let stdout_thread = std::thread::spawn(move || {
                let mut prometheus_port_tx = Some(prometheus_port_tx);

                let mut own_stderr = stderr();
                for line in BufReader::new(child_stdout).lines() {
                    let line = line.unwrap();
                    writeln!(own_stderr, "[{pid:08}][STDOUT] {line}").unwrap();
                    writeln!(stdout_file, "{line}").unwrap();

                    if let Some(tx) = prometheus_port_tx.as_mut() {
                        if let Some(captures) = METRICS_PORT_REGEX.captures(&line) {
                            let port: u16 = captures.get(1).unwrap().as_str().parse().unwrap();
                            let _ = tx.send(port);

                            prometheus_port_tx = None;
                        }
                    }
                }
            });

            let child_stderr = child.stderr.take().unwrap();
            let stderr_thread = std::thread::spawn(move || {
                let mut own_stderr = stderr();
                for line in BufReader::new(child_stderr).lines() {
                    let line = line.unwrap();
                    writeln!(own_stderr, "[{pid:08}][STDERR] {line}").unwrap();
                    writeln!(stderr_file, "{line}").unwrap();
                }
            });

            tx.send(pid).unwrap();

            let _ = terminate_rx.recv();

            info!(?pid, "Terminating a storage service process");

            let _ = child.kill();
            let _ = child.wait();

            stdout_thread.join().expect("failed to join stdout thread");
            stderr_thread.join().expect("failed to join stderr thread");
        });

        let pid = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("child process did not respond in time");
        let ports = read_ports(pid, None).expect("failed to read the service ports file");

        info!(?ports, ?pid, "Storage service process started");

        Self {
            grpc_port: ports.grpc_port,
            rest_port: ports.rest_port,
            metrics_port: prometheus_port_rx.recv().expect("send half has unexpectedly closed"),
            launch_thread: ManuallyDrop::new(launch_thread),
            terminate_tx,
        }
    }

    /// Return the grpc endpoint address.
    pub fn grpc_endpoint(&self) -> String {
        format!("http://127.0.0.1:{}", self.grpc_port)
    }

    /// Return the rest endpoint address.
    pub fn rest_endpoint(&self) -> String {
        format!("http://127.0.0.1:{}", self.rest_port)
    }

    /// Return the metrics endpoint address.
    pub fn metrics_endpoint(&self) -> String {
        format!("http://127.0.0.1:{}/metrics", self.metrics_port)
    }
}

impl Drop for ServiceHandle {
    fn drop(&mut self) {
        let launch_thread = unsafe { ManuallyDrop::take(&mut self.launch_thread) };
        let _ = self.terminate_tx.send(());
        let _ = launch_thread.join();
    }
}

fn get_process_env(env: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>) -> Vec<(String, String)> {
    let default_env = [
        ("OTEL_TRACES_EXPORTER", "none"),
        ("OTEL_LOGS_EXPORTER", "none"),
        ("OTEL_METRICS_EXPORTER", "prometheus"),
        ("OTEL_EXPORTER_PROMETHEUS_HOST", "0.0.0.0"),
        ("OTEL_EXPORTER_PROMETHEUS_PORT", "0"),
    ];

    default_env
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .chain(env.into_iter().map(|(k, v)| (k.into(), v.into())))
        .collect()
}
