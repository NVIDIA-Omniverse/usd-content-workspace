// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use axum::{body::Body, response::Response};
use http::StatusCode;
use storage_core_util::{error_limited, trace_limited};

use super::error::ProblemDetails;

// Helper method for common JSON response building
pub(crate) fn json_response<S: serde::Serialize + std::fmt::Debug>(
    status: StatusCode,
    data: &S,
) -> Result<Response, ProblemDetails> {
    trace_limited!(?data);
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(data).map_err(|e| {
            error_limited!(?e, "Failed to serialize response");
            ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR)
        })?))
        .map_err(|e| {
            error_limited!(?e, "Failed to build response");
            ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR)
        })
}

// Helper for empty responses
pub(crate) fn empty_response(status: StatusCode) -> Result<Response, ProblemDetails> {
    Response::builder().status(status).body(Body::empty()).map_err(|e| {
        error_limited!(?e, "Failed to build response");
        ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR)
    })
}
