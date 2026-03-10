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

use super::{authenticate_session, connect_session, disconnect_session, verify_host_key, SshError};
use crate::domain::execution_target::SshTarget;
use ssh2::Session;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(1);
const MIN_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(2);
const MAX_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
const MAX_IDLE_CONNECTIONS_PER_TARGET: usize = 4;

pub(super) struct SshConnectionPool {
    state: Arc<PoolState>,
}

pub(super) struct SshConnectionLease {
    pool: Arc<PoolState>,
    target: SshTarget,
    session: Option<Session>,
    reused: bool,
    discard_on_drop: bool,
}

struct PoolState {
    idle_connections: Mutex<HashMap<SshTarget, Vec<IdleSshConnection>>>,
}

struct IdleSshConnection {
    session: Session,
    idle_timeout: Duration,
    keepalive_interval: Option<Duration>,
    last_used_at: Instant,
    next_keepalive_at: Instant,
}

impl SshConnectionPool {
    pub(super) fn new() -> Self {
        let state = Arc::new(PoolState {
            idle_connections: Mutex::new(HashMap::new()),
        });
        spawn_maintenance_thread(&state);

        Self { state }
    }

    pub(super) fn checkout(&self, target: &SshTarget) -> Result<SshConnectionLease, SshError> {
        if let Some(session) = self.state.take_idle_session(target) {
            return Ok(SshConnectionLease::new(
                self.state.clone(),
                target.clone(),
                session,
                true,
            ));
        }

        self.checkout_fresh(target)
    }

    pub(super) fn checkout_fresh(
        &self,
        target: &SshTarget,
    ) -> Result<SshConnectionLease, SshError> {
        let session = create_authenticated_session(target)?;
        Ok(SshConnectionLease::new(
            self.state.clone(),
            target.clone(),
            session,
            false,
        ))
    }
}

impl SshConnectionLease {
    fn new(pool: Arc<PoolState>, target: SshTarget, session: Session, reused: bool) -> Self {
        Self {
            pool,
            target,
            session: Some(session),
            reused,
            discard_on_drop: false,
        }
    }

    pub(super) fn session(&self) -> &Session {
        self.session
            .as_ref()
            .expect("ssh session lease must hold a session until drop")
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
        let Some(session) = self.session.take() else {
            return;
        };

        if self.discard_on_drop {
            disconnect_session(session);
        } else {
            self.pool.return_idle_session(self.target.clone(), session);
        }
    }
}

impl PoolState {
    fn take_idle_session(&self, target: &SshTarget) -> Option<Session> {
        let now = Instant::now();
        let mut stale_sessions = Vec::new();
        let mut idle_connections = self
            .idle_connections
            .lock()
            .expect("ssh connection pool mutex should not be poisoned");
        let session = idle_connections.get_mut(target).and_then(|connections| {
            while let Some(connection) = connections.pop() {
                if connection.is_expired(now) {
                    stale_sessions.push(connection.session);
                    continue;
                }

                return Some(connection.session);
            }

            None
        });
        idle_connections.retain(|_, connections| !connections.is_empty());
        drop(idle_connections);

        for session in stale_sessions {
            disconnect_session(session);
        }

        session
    }

    fn return_idle_session(&self, target: SshTarget, session: Session) {
        let connection = IdleSshConnection::new(session, target.connection_idle_timeout);
        let mut disconnected_sessions = Vec::new();
        let mut idle_connections = self
            .idle_connections
            .lock()
            .expect("ssh connection pool mutex should not be poisoned");
        let connections = idle_connections.entry(target).or_default();
        connections.push(connection);

        for _ in 0..excess_idle_connection_count(connections.len()) {
            let Some(index) = oldest_connection_index(connections) else {
                break;
            };
            disconnected_sessions.push(connections.swap_remove(index).session);
        }

        drop(idle_connections);

        for session in disconnected_sessions {
            disconnect_session(session);
        }
    }

    fn maintain_idle_sessions(&self) {
        let now = Instant::now();
        let mut disconnected_sessions = Vec::new();
        let mut idle_connections = self
            .idle_connections
            .lock()
            .expect("ssh connection pool mutex should not be poisoned");

        for connections in idle_connections.values_mut() {
            let mut index = 0;
            while index < connections.len() {
                if connections[index].is_expired(now) {
                    disconnected_sessions.push(connections.swap_remove(index).session);
                    continue;
                }

                if connections[index].should_send_keepalive(now) {
                    if connections[index].session.keepalive_send().is_err() {
                        connections[index].record_keepalive(now);
                        index += 1;
                        continue;
                    }

                    connections[index].record_keepalive(now);
                }

                index += 1;
            }
        }

        idle_connections.retain(|_, connections| !connections.is_empty());
        drop(idle_connections);

        for session in disconnected_sessions {
            disconnect_session(session);
        }
    }
}

impl IdleSshConnection {
    fn new(session: Session, idle_timeout: Duration) -> Self {
        let keepalive_interval = keepalive_interval_for(idle_timeout);
        let now = Instant::now();

        Self {
            session,
            idle_timeout,
            keepalive_interval,
            last_used_at: now,
            next_keepalive_at: now + keepalive_interval.unwrap_or(idle_timeout),
        }
    }

    fn is_expired(&self, now: Instant) -> bool {
        now.duration_since(self.last_used_at) >= self.idle_timeout
    }

    fn should_send_keepalive(&self, now: Instant) -> bool {
        self.keepalive_interval.is_some() && now >= self.next_keepalive_at
    }

    fn record_keepalive(&mut self, now: Instant) {
        if let Some(interval) = self.keepalive_interval {
            self.next_keepalive_at = now + interval;
        }
    }
}

fn create_authenticated_session(target: &SshTarget) -> Result<Session, SshError> {
    let session = connect_session(target)?;
    verify_host_key(&session, target)?;
    authenticate_session(&session, target)?;
    let keepalive_interval = keepalive_interval_for(target.connection_idle_timeout)
        .map(|interval| interval.as_secs().min(u64::from(u32::MAX)) as u32)
        .unwrap_or(0);
    session.set_keepalive(false, keepalive_interval);
    Ok(session)
}

fn spawn_maintenance_thread(state: &Arc<PoolState>) {
    let state = Arc::downgrade(state);
    thread::spawn(move || run_maintenance_loop(state));
}

fn run_maintenance_loop(state: Weak<PoolState>) {
    loop {
        thread::sleep(MAINTENANCE_INTERVAL);

        let Some(state) = state.upgrade() else {
            break;
        };

        state.maintain_idle_sessions();
    }
}

fn keepalive_interval_for(idle_timeout: Duration) -> Option<Duration> {
    if idle_timeout < MIN_KEEPALIVE_INTERVAL.saturating_mul(2) {
        return None;
    }

    let candidate = idle_timeout / 3;
    Some(candidate.clamp(MIN_KEEPALIVE_INTERVAL, MAX_KEEPALIVE_INTERVAL))
}

fn excess_idle_connection_count(len: usize) -> usize {
    len.saturating_sub(MAX_IDLE_CONNECTIONS_PER_TARGET)
}

fn oldest_connection_index(connections: &[IdleSshConnection]) -> Option<usize> {
    connections
        .iter()
        .enumerate()
        .min_by_key(|(_, connection)| connection.last_used_at)
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn idle_pool_cap_only_trims_connections_above_limit() {
        assert_eq!(excess_idle_connection_count(0), 0);
        assert_eq!(excess_idle_connection_count(4), 0);
        assert_eq!(excess_idle_connection_count(6), 2);
    }
}
