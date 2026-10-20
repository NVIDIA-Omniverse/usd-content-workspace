<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# AI coding agents

The [skills](../skills/README.md) directory contains the `usd-content-workspace-expert` skill, which helps AI coding agents — and people — choose the services a workflow needs, and connect to, deploy, configure, validate, or implement each one. It records which version each answer applies to, treats every service as a replaceable reference implementation, and covers Kubernetes, local-cluster, and local-process deployments. It routes to the [local-process-stack](../skills/local-process-stack/SKILL.md) skill and to the skills released with the components: the Storage API contract map, the Storage Service documentation map, and the User Info Service documentation map. Open this repository in an agent that reads [AGENTS.md](../AGENTS.md); the skills route to paths in this repository, so keep it checked out if you point an agent elsewhere at them.

The expert answers questions from other agents as well as from people. An orchestrating agent can ask which services a workload needs, the order to stand them up, what a Helm value or API field does, or how to verify a deployment.

Agents working in this repository explain and recommend by default; they build, install, or deploy only when asked.
