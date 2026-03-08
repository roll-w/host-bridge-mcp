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

use crate::application::execution_service::OutputKind;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use uuid::Uuid;

pub(super) struct OutputAccumulator {
    stdout: OutputSpool,
    stderr: OutputSpool,
}

impl OutputAccumulator {
    pub(super) fn new() -> io::Result<Self> {
        Ok(Self {
            stdout: OutputSpool::new("stdout")?,
            stderr: OutputSpool::new("stderr")?,
        })
    }

    pub(super) fn push(&mut self, stream: &OutputKind, text: &str) {
        let spool = match stream {
            OutputKind::Stdout => &mut self.stdout,
            OutputKind::Stderr => &mut self.stderr,
        };

        spool.append(text);
    }

    pub(super) fn finish(self) -> (String, String) {
        (self.stdout.read_all(), self.stderr.read_all())
    }
}

struct OutputSpool {
    path: PathBuf,
    file: File,
}

impl OutputSpool {
    fn new(prefix: &str) -> io::Result<Self> {
        let path =
            std::env::temp_dir().join(format!("host-bridge-mcp-{prefix}-{}.log", Uuid::new_v4()));
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)?;

        Ok(Self { path, file })
    }

    fn append(&mut self, text: &str) {
        let _ = self.file.write_all(text.as_bytes());
    }

    fn read_all(mut self) -> String {
        let _ = self.file.flush();
        fs::read_to_string(&self.path).unwrap_or_default()
    }
}

impl Drop for OutputSpool {
    fn drop(&mut self) {
        let _ = self.file.flush();
        let _ = fs::remove_file(&self.path);
    }
}
