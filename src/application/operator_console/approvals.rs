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

use crate::application::execution_service::ConfirmationRequest;
use crate::application::operator_console::OperatorConsole;
use tokio::sync::oneshot;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct SessionApprovalKey {
    session_id: String,
    scope: ExecutionApprovalScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ExecutionApprovalScope {
    // This scope covers execution semantics; per-request limits and output rendering are separate.
    server: String,
    platform: String,
    command_line: String,
    executable: String,
    args: Vec<String>,
    working_directory: Option<String>,
    env: Vec<(String, String)>,
    contains_shell_operator: bool,
}

impl ExecutionApprovalScope {
    fn from_request(request: &ConfirmationRequest) -> Self {
        let mut env = request
            .env
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Vec<_>>();
        env.sort();

        Self {
            server: request.server.clone(),
            platform: request.platform.clone(),
            command_line: request.command_line.clone(),
            executable: request.executable.clone(),
            args: request.args.clone(),
            working_directory: request.working_directory.clone(),
            env,
            contains_shell_operator: request.contains_shell_operator,
        }
    }
}

impl SessionApprovalKey {
    pub(super) fn new(session_id: &str, request: &ConfirmationRequest) -> Self {
        Self {
            session_id: session_id.to_string(),
            scope: ExecutionApprovalScope::from_request(request),
        }
    }
}

pub(super) struct PendingApproval {
    pub(super) id: Uuid,
    pub(super) execution_id: Uuid,
    pub(super) request: ConfirmationRequest,
    pub(super) created_at: String,
    session_approval_key: Option<SessionApprovalKey>,
    responder: Option<oneshot::Sender<bool>>,
}

pub(super) struct PendingApprovalGuard {
    console: OperatorConsole,
    approval_id: Uuid,
    active: bool,
}

impl PendingApproval {
    pub(super) fn new(
        id: Uuid,
        execution_id: Uuid,
        request: ConfirmationRequest,
        responder: oneshot::Sender<bool>,
        session_approval_key: Option<SessionApprovalKey>,
    ) -> Self {
        Self {
            id,
            execution_id,
            request,
            created_at: super::current_console_timestamp(),
            session_approval_key,
            responder: Some(responder),
        }
    }

    pub(super) fn session_approval_key(&self) -> Option<&SessionApprovalKey> {
        self.session_approval_key.as_ref()
    }

    pub(super) fn deliver(&mut self, approved: bool) {
        if let Some(sender) = self.responder.take() {
            let _ = sender.send(approved);
        }
    }

    pub(super) fn cancel(&mut self) {
        self.responder.take();
    }
}

impl PendingApprovalGuard {
    pub(super) fn new(console: OperatorConsole, approval_id: Uuid) -> Self {
        Self {
            console,
            approval_id,
            active: true,
        }
    }

    pub(super) fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for PendingApprovalGuard {
    fn drop(&mut self) {
        if self.active {
            self.console.cancel_pending_confirmation(self.approval_id);
        }
    }
}
