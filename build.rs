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

use std::env;
use std::path::Path;
use std::process::{Command, ExitStatus};

fn main() {
    let manifest_dir = env::var_os("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR must be set for the web build");
    let web_dir = Path::new(&manifest_dir).join("web");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=web/index.html");
    println!("cargo:rerun-if-changed=web/package.json");
    println!("cargo:rerun-if-changed=web/package-lock.json");
    println!("cargo:rerun-if-changed=web/tsconfig.json");
    println!("cargo:rerun-if-changed=web/tsconfig.app.json");
    println!("cargo:rerun-if-changed=web/tsconfig.node.json");
    println!("cargo:rerun-if-changed=web/vite.config.ts");
    println!("cargo:rerun-if-changed=web/src");

    if !web_dir.join("node_modules").is_dir() {
        panic!(
            "web dependencies are missing; run `npm ci` in {} before building Rust",
            web_dir.display()
        );
    }

    let status = run_npm(&web_dir, &["run", "build"]).unwrap_or_else(|error| {
        panic!(
            "failed to start the frontend build in {}: {error}",
            web_dir.display()
        )
    });
    if !status.success() {
        panic!(
            "frontend build failed in {} with status {status}",
            web_dir.display()
        );
    }
}

fn run_npm(web_dir: &Path, args: &[&str]) -> std::io::Result<ExitStatus> {
    let executable = if cfg!(windows) { "npm.cmd" } else { "npm" };
    Command::new(executable)
        .args(args)
        .current_dir(web_dir)
        .status()
}
