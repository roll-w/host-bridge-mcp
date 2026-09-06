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

use clap::Parser;
use std::ffi::OsString;

#[derive(Debug, Clone, Parser, PartialEq, Eq)]
#[command(
    version,
    about = "Run the host-bridge MCP server and its local operator interfaces",
    long_about = None
)]
pub struct CliOptions {
    /// Set the configuration file path.
    #[arg(short, long = "config", value_name = "PATH", value_parser = parse_non_empty)]
    pub config_path: Option<String>,

    /// Override the HTTP bind host.
    #[arg(long, value_name = "HOST", value_parser = parse_non_empty)]
    pub host: Option<String>,

    /// Override the HTTP bind port.
    #[arg(long, value_name = "PORT", value_parser = parse_port)]
    pub port: Option<u16>,

    /// Enable the operator TUI. A bare option enables it; otherwise the config value is used.
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    pub tui: Option<bool>,

    /// Open the web console at startup. A bare option enables it; otherwise the config value is used.
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    pub web: Option<bool>,
}

pub fn parse_args<I, T>(args: I) -> Result<CliOptions, clap::Error>
where
    I: IntoIterator<Item=T>,
    T: Into<OsString> + Clone,
{
    CliOptions::try_parse_from(args)
}

fn parse_non_empty(value: &str) -> Result<String, String> {
    if value.trim().is_empty() {
        return Err("value must not be empty".to_string());
    }
    Ok(value.to_string())
}

fn parse_port(value: &str) -> Result<u16, String> {
    let port = value
        .parse::<u16>()
        .map_err(|_| "port must be an integer from 1 to 65535".to_string())?;
    if port == 0 {
        return Err("port must be greater than zero".to_string());
    }
    Ok(port)
}

pub fn resolve_bind_address(
    configured: &str,
    host_override: Option<&str>,
    port_override: Option<u16>,
) -> Result<String, String> {
    if host_override.is_none() && port_override.is_none() {
        return Ok(configured.to_string());
    }

    let (configured_host, configured_port) = split_bind_address(configured)?;
    let host = host_override.unwrap_or(configured_host.as_str());
    let port = port_override.unwrap_or(configured_port);
    format_bind_address(host, port)
}

fn split_bind_address(configured: &str) -> Result<(String, u16), String> {
    if let Some(end) = configured
        .strip_prefix('[')
        .and_then(|value| value.find(']'))
    {
        let host = configured[1..end + 1].to_string();
        let port = configured
            .get(end + 2..)
            .and_then(|value| value.strip_prefix(':'))
            .ok_or_else(|| format!("configured server.address '{configured}' has no port"))?
            .parse::<u16>()
            .map_err(|_| format!("configured server.address '{configured}' has an invalid port"))?;
        if port == 0 {
            return Err(format!(
                "configured server.address '{configured}' has an invalid port"
            ));
        }
        return Ok((host, port));
    }

    let (host, port) = configured
        .rsplit_once(':')
        .ok_or_else(|| format!("configured server.address '{configured}' has no port"))?;
    if host.is_empty() {
        return Err(format!(
            "configured server.address '{configured}' has no host"
        ));
    }
    let port = port
        .parse::<u16>()
        .map_err(|_| format!("configured server.address '{configured}' has an invalid port"))?;
    if port == 0 {
        return Err(format!(
            "configured server.address '{configured}' has an invalid port"
        ));
    }
    Ok((host.to_string(), port))
}

fn format_bind_address(host: &str, port: u16) -> Result<String, String> {
    if host.trim().is_empty() {
        return Err("bind host must not be empty".to_string());
    }

    if host.contains(':') && !host.starts_with('[') {
        Ok(format!("[{host}]:{port}"))
    } else {
        Ok(format!("{host}:{port}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        std::iter::once("host-bridge-mcp".to_string())
            .chain(values.iter().map(|value| (*value).to_string()))
            .collect()
    }

    #[test]
    fn omitted_interface_options_use_the_config_values() {
        let options = parse_args(args(&[])).expect("default CLI should parse");

        assert_eq!(options.tui, None);
        assert_eq!(options.web, None);
        assert_eq!(options.config_path, None);
        assert_eq!(options.host, None);
        assert_eq!(options.port, None);
    }

    #[test]
    fn boolean_flags_accept_bare_and_explicit_values() {
        let options = parse_args(args(&["--tui", "false", "--web=false"]))
            .expect("boolean flags should parse");

        assert_eq!(options.tui, Some(false));
        assert_eq!(options.web, Some(false));

        let options =
            parse_args(args(&["--tui", "--web"])).expect("bare boolean flags should parse");
        assert_eq!(options.tui, Some(true));
        assert_eq!(options.web, Some(true));
    }

    #[test]
    fn host_and_port_overrides_parse() {
        let options = parse_args(args(&["--host", "0.0.0.0", "--port=9000"]))
            .expect("bind overrides should parse");

        assert_eq!(options.host.as_deref(), Some("0.0.0.0"));
        assert_eq!(options.port, Some(9000));
    }

    #[test]
    fn config_path_keeps_the_existing_long_option() {
        let options =
            parse_args(args(&["--config", "custom.yaml"])).expect("config path should parse");

        assert_eq!(options.config_path.as_deref(), Some("custom.yaml"));
    }

    #[test]
    fn bind_address_override_preserves_the_other_component() {
        assert_eq!(
            resolve_bind_address("127.0.0.1:8787", Some("0.0.0.0"), None).unwrap(),
            "0.0.0.0:8787"
        );
        assert_eq!(
            resolve_bind_address("127.0.0.1:8787", None, Some(9000)).unwrap(),
            "127.0.0.1:9000"
        );
        assert_eq!(
            resolve_bind_address("[::1]:8787", Some("::"), Some(9000)).unwrap(),
            "[::]:9000"
        );
    }

    #[test]
    fn invalid_port_is_rejected() {
        let error = parse_args(args(&["--port", "0"]))
            .expect_err("zero port should be rejected")
            .to_string();
        assert!(error.contains("port must be greater than zero"));
    }
}
