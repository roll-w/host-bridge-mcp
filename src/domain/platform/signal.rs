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

use std::io;

#[cfg(unix)]
pub async fn wait_for_termination_signal() -> io::Result<&'static str> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sigterm = signal(SignalKind::terminate())?;
    tokio::select! {
        _ = sigint.recv() => Ok("SIGINT"),
        _ = sigterm.recv() => Ok("SIGTERM"),
    }
}

#[cfg(not(unix))]
pub async fn wait_for_termination_signal() -> io::Result<&'static str> {
    tokio::signal::ctrl_c().await?;
    Ok("CTRL_C")
}
