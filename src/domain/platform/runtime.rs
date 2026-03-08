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

use crate::config::TargetPlatform;
use std::fs;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimePlatform {
    Windows,
    Linux,
    Macos,
}

impl RuntimePlatform {
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else {
            Self::Linux
        }
    }

    pub fn is_windows(self) -> bool {
        matches!(self, Self::Windows)
    }
}

pub fn resolve_target_platform(target: TargetPlatform) -> RuntimePlatform {
    match target {
        TargetPlatform::Auto => RuntimePlatform::current(),
        TargetPlatform::Windows => RuntimePlatform::Windows,
        TargetPlatform::Linux => RuntimePlatform::Linux,
        TargetPlatform::Macos => RuntimePlatform::Macos,
    }
}

pub fn current_is_wsl() -> bool {
    if RuntimePlatform::current() != RuntimePlatform::Linux {
        return false;
    }

    match fs::read_to_string("/proc/sys/kernel/osrelease") {
        Ok(value) => value.to_ascii_lowercase().contains("microsoft"),
        Err(_) => false,
    }
}
