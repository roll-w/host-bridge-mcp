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

pub(super) struct PendingApproval {
    pub(super) id: Uuid,
    pub(super) request: ConfirmationRequest,
    pub(super) created_at: String,
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
        request: ConfirmationRequest,
        responder: oneshot::Sender<bool>,
    ) -> Self {
        Self {
            id,
            request,
            created_at: super::current_console_timestamp(),
            responder: Some(responder),
        }
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
