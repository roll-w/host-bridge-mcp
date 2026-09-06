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

use super::HttpState;
use super::api_handlers::{execution_error_to_api, parse_execution_id};
use crate::application::execution_service::ExecutionEvent;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use std::convert::Infallible;
use std::time::Duration;
use tokio_stream::StreamExt;
use tokio_stream::once;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

pub(crate) async fn stream_execution(
    Path(execution_id): Path<String>,
    State(state): State<HttpState>,
) -> Result<impl IntoResponse, crate::transport::api::ApiError> {
    let execution_id = parse_execution_id(&execution_id)?;
    let subscription = state
        .execution_service
        .subscribe(execution_id)
        .await
        .map_err(execution_error_to_api)?;

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

    Ok(Sse::new(initial_stream.chain(updates)).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

fn to_sse_event(event: &ExecutionEvent) -> Event {
    Event::default()
        .event(event_name(event))
        .data(serialize_event(event))
}

fn lagged_event(skipped: u64) -> Event {
    Event::default()
        .event("lagged")
        .data(serde_json::json!({ "type": "lagged", "skipped": skipped }).to_string())
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
        serde_json::json!({
            "type": "error",
            "message": format!("failed to serialize event: {error}")
        })
            .to_string()
    })
}
