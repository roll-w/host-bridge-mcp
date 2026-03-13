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

use super::{SshError, SshSessionHandle, create_authenticated_session};
use crate::domain::execution_target::SshTarget;
use russh::Disconnect;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;

const SSH_DISCONNECT_LANGUAGE: &str = "en-US";

pub(super) struct SshConnectionManager {
    state: Arc<ManagerState>,
}

pub(super) struct SshConnectionLease {
    state: Arc<ManagerState>,
    target: SshTarget,
    handle: Option<SshSessionHandle>,
    reused: bool,
    discard_on_drop: bool,
}

struct ManagerState {
    idle_connections: Mutex<HashMap<SshTarget, ManagedSshConnection>>,
    next_generation: AtomicU64,
}

struct ManagedSshConnection {
    generation: u64,
    handle: SshSessionHandle,
    idle_timeout: Duration,
    returned_at: Instant,
    cleanup_task: Option<JoinHandle<()>>,
}

impl SshConnectionManager {
    pub(super) fn new() -> Self {
        Self {
            state: Arc::new(ManagerState {
                idle_connections: Mutex::new(HashMap::new()),
                next_generation: AtomicU64::new(1),
            }),
        }
    }

    pub(super) async fn checkout(&self, target: SshTarget) -> Result<SshConnectionLease, SshError> {
        if let Some(connection) = self.state.take_connection(&target) {
            if connection.is_reusable(Instant::now()) {
                return Ok(SshConnectionLease::new(
                    self.state.clone(),
                    target,
                    connection.handle,
                    true,
                ));
            }

            if !connection.handle.is_closed() {
                schedule_disconnect(connection.handle, "stale cached connection");
            }
        }

        self.checkout_fresh(target).await
    }

    pub(super) async fn checkout_fresh(
        &self,
        target: SshTarget,
    ) -> Result<SshConnectionLease, SshError> {
        if let Some(connection) = self.state.take_connection(&target) {
            if !connection.handle.is_closed() {
                schedule_disconnect(connection.handle, "connection refresh");
            }
        }

        let handle = create_authenticated_session(target.clone()).await?;
        Ok(SshConnectionLease::new(
            self.state.clone(),
            target,
            handle,
            false,
        ))
    }
}

impl SshConnectionLease {
    fn new(
        state: Arc<ManagerState>,
        target: SshTarget,
        handle: SshSessionHandle,
        reused: bool,
    ) -> Self {
        Self {
            state,
            target,
            handle: Some(handle),
            reused,
            discard_on_drop: false,
        }
    }

    pub(super) fn handle(&self) -> &SshSessionHandle {
        self.handle
            .as_ref()
            .expect("ssh connection lease must hold a handle until drop")
    }

    pub(super) fn discard(&mut self) {
        self.discard_on_drop = true;
    }

    pub(super) fn was_reused(&self) -> bool {
        self.reused
    }
}

impl Drop for SshConnectionLease {
    fn drop(&mut self) {
        let Some(handle) = self.handle.take() else {
            return;
        };

        if self.discard_on_drop {
            schedule_disconnect(handle, "connection discarded");
        } else {
            self.state.return_connection(self.target.clone(), handle);
        }
    }
}

impl ManagerState {
    fn take_connection(&self, target: &SshTarget) -> Option<ManagedSshConnection> {
        let mut connection = self
            .idle_connections
            .lock()
            .expect("ssh connection manager mutex should not be poisoned")
            .remove(target)?;
        connection.abort_cleanup();
        Some(connection)
    }

    fn return_connection(self: &Arc<Self>, target: SshTarget, handle: SshSessionHandle) {
        if handle.is_closed() {
            return;
        }

        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        let idle_timeout = target.connection_idle_timeout;
        let mut connection = ManagedSshConnection::new(generation, handle, idle_timeout);
        connection.cleanup_task = spawn_idle_cleanup_task(
            target.clone(),
            generation,
            idle_timeout,
            Arc::downgrade(self),
        );
        let replaced = self
            .idle_connections
            .lock()
            .expect("ssh connection manager mutex should not be poisoned")
            .insert(target.clone(), connection);

        if let Some(replaced) = replaced {
            let mut replaced = replaced;
            replaced.abort_cleanup();
            if !replaced.handle.is_closed() {
                schedule_disconnect(replaced.handle, "connection superseded");
            }
        }
    }

    fn remove_if_generation(
        &self,
        target: &SshTarget,
        generation: u64,
    ) -> Option<SshSessionHandle> {
        let mut idle_connections = self
            .idle_connections
            .lock()
            .expect("ssh connection manager mutex should not be poisoned");
        let should_remove = idle_connections
            .get(target)
            .is_some_and(|connection| connection.generation == generation);
        if !should_remove {
            return None;
        }

        idle_connections
            .remove(target)
            .map(|connection| connection.handle)
    }
}

impl ManagedSshConnection {
    fn new(generation: u64, handle: SshSessionHandle, idle_timeout: Duration) -> Self {
        Self {
            generation,
            handle,
            idle_timeout,
            returned_at: Instant::now(),
            cleanup_task: None,
        }
    }

    fn is_reusable(&self, now: Instant) -> bool {
        !self.handle.is_closed() && !is_connection_expired(self.returned_at, self.idle_timeout, now)
    }

    fn abort_cleanup(&mut self) {
        if let Some(cleanup_task) = self.cleanup_task.take() {
            cleanup_task.abort();
        }
    }
}

fn spawn_idle_cleanup_task(
    target: SshTarget,
    generation: u64,
    idle_timeout: Duration,
    state: std::sync::Weak<ManagerState>,
) -> Option<JoinHandle<()>> {
    spawn_background_task(async move {
        tokio::time::sleep(idle_timeout).await;

        let Some(state) = state.upgrade() else {
            return;
        };
        let Some(handle) = state.remove_if_generation(&target, generation) else {
            return;
        };

        schedule_disconnect(handle, "idle timeout");
    })
}

fn schedule_disconnect(handle: SshSessionHandle, description: &'static str) {
    if handle.is_closed() {
        return;
    }

    spawn_background_task(async move {
        let _ = handle
            .disconnect(
                Disconnect::ByApplication,
                description,
                SSH_DISCONNECT_LANGUAGE,
            )
            .await;
    });
}

fn spawn_background_task<F>(future: F) -> Option<JoinHandle<()>>
where
    F: Future<Output = ()> + Send + 'static,
{
    if let Ok(runtime_handle) = tokio::runtime::Handle::try_current() {
        return Some(runtime_handle.spawn(future));
    }

    None
}

fn is_connection_expired(returned_at: Instant, idle_timeout: Duration, now: Instant) -> bool {
    now.duration_since(returned_at) >= idle_timeout
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ssh::keepalive_interval_for;

    #[test]
    fn keepalive_interval_is_disabled_for_short_idle_timeout() {
        assert_eq!(keepalive_interval_for(Duration::from_secs(3)), None);
    }

    #[test]
    fn keepalive_interval_scales_with_idle_timeout() {
        assert_eq!(
            keepalive_interval_for(Duration::from_secs(12)),
            Some(Duration::from_secs(4))
        );
        assert_eq!(
            keepalive_interval_for(Duration::from_secs(600)),
            Some(Duration::from_secs(30))
        );
    }

    #[test]
    fn connection_expiration_uses_return_time_and_timeout() {
        let returned_at = Instant::now();
        assert!(!is_connection_expired(
            returned_at,
            Duration::from_secs(10),
            returned_at + Duration::from_secs(9)
        ));
        assert!(is_connection_expired(
            returned_at,
            Duration::from_secs(10),
            returned_at + Duration::from_secs(10)
        ));
    }
}
