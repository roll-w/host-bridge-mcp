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

use tokio::sync::watch;

#[derive(Clone)]
pub struct ShutdownController {
    sender: watch::Sender<bool>,
    receiver: watch::Receiver<bool>,
}

impl ShutdownController {
    pub fn request_shutdown(&self) -> bool {
        if *self.receiver.borrow() {
            return false;
        }

        self.sender.send(true).is_ok()
    }

    pub async fn wait_for_shutdown(&self) {
        let mut receiver = self.receiver.clone();
        if *receiver.borrow_and_update() {
            return;
        }

        while receiver.changed().await.is_ok() {
            if *receiver.borrow_and_update() {
                return;
            }
        }
    }

    pub fn is_shutdown_requested(&self) -> bool {
        *self.receiver.borrow()
    }
}

impl Default for ShutdownController {
    fn default() -> Self {
        let (sender, receiver) = watch::channel(false);
        Self { sender, receiver }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn request_shutdown_wakes_waiters() {
        let controller = ShutdownController::default();
        let waiter = controller.clone();

        let task = tokio::spawn(async move {
            waiter.wait_for_shutdown().await;
        });

        tokio::task::yield_now().await;
        assert!(controller.request_shutdown());
        task.await.expect("shutdown wait should complete");
        assert!(controller.is_shutdown_requested());
    }

    #[tokio::test]
    async fn wait_returns_after_prior_shutdown_request() {
        let controller = ShutdownController::default();
        assert!(controller.request_shutdown());

        controller.wait_for_shutdown().await;
        assert!(controller.is_shutdown_requested());
    }
}
