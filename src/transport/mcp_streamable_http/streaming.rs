/*
 * Copyright 2026-present RollW
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *        http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use crate::application::execution_service::{ExecutionError, ExecutionEvent};
use crate::transport::mcp_streamable_http::HttpState;
use axum::extract::{Path, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::{http::StatusCode, Json};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::time::Duration;
use tokio_stream::once;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use uuid::Uuid;

pub(super) async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

pub(super) async fn stream_execution(
    Path(execution_id): Path<String>,
    State(state): State<HttpState>,
) -> Result<impl IntoResponse, StatusCode> {
    let execution_id = Uuid::parse_str(&execution_id).map_err(|_| StatusCode::BAD_REQUEST)?;

    let subscription = state
        .execution_service
        .subscribe(execution_id)
        .await
        .map_err(|error| execution_error_to_status(&error))?;

    let initial_event =
        Event::default()
            .event("status")
            .data(serialize_event(&ExecutionEvent::Status {
                state: subscription.current_state,
                message: Some("Subscribed to execution stream".to_string()),
            }));

    let initial_stream = once(Ok::<Event, Infallible>(initial_event));
    let updates = BroadcastStream::new(subscription.receiver).filter_map(|event| match event {
        Ok(event) => Some(Ok::<Event, Infallible>(to_sse_event(&event))),
        Err(BroadcastStreamRecvError::Lagged(skipped)) => {
            Some(Ok::<Event, Infallible>(lagged_event(skipped)))
        }
    });

    let stream = initial_stream.chain(updates);
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

fn execution_error_to_status(error: &ExecutionError) -> StatusCode {
    match error {
        ExecutionError::NotFound(_) => StatusCode::NOT_FOUND,
        _ => StatusCode::BAD_REQUEST,
    }
}

fn to_sse_event(event: &ExecutionEvent) -> Event {
    Event::default()
        .event(event_name(event))
        .data(serialize_event(event))
}

fn lagged_event(skipped: u64) -> Event {
    Event::default().event("lagged").data(
        json!({
            "type": "lagged",
            "skipped": skipped,
        })
            .to_string(),
    )
}

fn event_name(event: &ExecutionEvent) -> &'static str {
    match event {
        ExecutionEvent::Status { .. } => "status",
        ExecutionEvent::Output { .. } => "output",
        ExecutionEvent::Exit { .. } => "exit",
        ExecutionEvent::Error { .. } => "error",
    }
}

fn serialize_event(event: &ExecutionEvent) -> String {
    serde_json::to_string(event).unwrap_or_else(|error| {
        json!({
            "type": "error",
            "message": format!("failed to serialize event: {error}")
        })
            .to_string()
    })
}
