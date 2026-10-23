///////////////////////////////////////////////////////////////////////////////
//
//  Copyright 2018-2026 Robonomics Network <research@robonomics.network>
//
//  Licensed under the Apache License, Version 2.0 (the "License");
//  you may not use this file except in compliance with the License.
//  You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.
//
///////////////////////////////////////////////////////////////////////////////
//! `edge config` — configuration validation and inspection.

use super::{CliError, CliResult};
use crate::config::{Config, DEFAULT_CONFIG_PATH};
use clap::Subcommand;
use std::path::PathBuf;

/// `edge config` subcommands.
#[derive(Debug, Subcommand)]
pub(crate) enum ConfigCommand {
    /// Load and validate a configuration file, reporting the first error.
    Check {
        /// Path to the gateway TOML file.
        #[arg(short, long, default_value = DEFAULT_CONFIG_PATH)]
        config: PathBuf,
    },
    /// Print the normalised, validated configuration as TOML.
    ///
    /// The gateway only stores *paths* to secret material (never the secrets
    /// themselves), so nothing sensitive is emitted.
    Print {
        /// Path to the gateway TOML file.
        #[arg(short, long, default_value = DEFAULT_CONFIG_PATH)]
        config: PathBuf,
    },
    /// Write a default configuration to stdout (or a file with `--output`).
    ///
    /// The generated file documents every section with the built-in defaults,
    /// giving operators a validated starting point to edit.
    Generate {
        /// Optional destination path; writes to stdout when omitted.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Overwrite the destination file if it already exists.
        #[arg(long)]
        force: bool,
    },
}

/// Dispatch a `config` subcommand.
pub(crate) fn run(command: ConfigCommand) -> CliResult {
    match command {
        ConfigCommand::Check { config } => check(config),
        ConfigCommand::Print { config } => print(config),
        ConfigCommand::Generate { output, force } => generate(output, force),
    }
}

/// Load + validate the file, mapping any error to exit code `2`.
fn load(path: PathBuf) -> Result<Config, CliError> {
    Config::from_file(&path).map_err(|e| CliError::usage(e.to_string()))
}

/// Implements `edge config check`.
///
/// A configuration that validates but exposes an unauthenticated public HTTP
/// ingress (see [`Config::http_open_relay`]) is still reported as valid, with
/// a warning on `stderr`.
fn check(path: PathBuf) -> CliResult {
    let config = load(path)?;
    if let Some(warning) = config.http_open_relay_warning() {
        eprintln!("warning: {warning}");
    }
    println!("configuration is valid");
    Ok(())
}

/// Implements `edge config print`.
fn print(path: PathBuf) -> CliResult {
    let config = load(path)?;
    let rendered = toml::to_string_pretty(&config)
        .map_err(|e| CliError::runtime(format!("failed to serialize config: {e}")))?;
    print!("{rendered}");
    Ok(())
}

/// Implements `edge config generate`.
///
/// Renders [`Config::default`] as TOML and either writes it to `stdout` or to a
/// file. The rendered defaults are re-validated so the emitted file is always a
/// valid starting point, and the `[http]` section is annotated with the ingress
/// route (serde-based TOML rendering cannot carry comments itself).
fn generate(output: Option<PathBuf>, force: bool) -> CliResult {
    let config = Config::default();
    config
        .validate()
        .map_err(|e| CliError::runtime(format!("default configuration is invalid: {e}")))?;
    let rendered = toml::to_string_pretty(&config)
        .map_err(|e| CliError::runtime(format!("failed to serialize config: {e}")))?;
    let rendered = annotate_http_section(&rendered);

    match output {
        None => {
            print!("{rendered}");
            Ok(())
        }
        Some(path) => {
            if path.exists() && !force {
                return Err(CliError::usage(format!(
                    "{} already exists; pass --force to overwrite",
                    path.display()
                )));
            }
            std::fs::write(&path, rendered).map_err(|e| {
                CliError::runtime(format!("failed to write {}: {e}", path.display()))
            })?;
            eprintln!("wrote default configuration to {}", path.display());
            Ok(())
        }
    }
}

/// Prepend a comment block documenting the ingress route to the `[http]`
/// table header of a rendered configuration.
///
/// Returns the input unchanged if no `[http]` header is present.
fn annotate_http_section(rendered: &str) -> String {
    let comment = format!(
        "# HTTP ingress: {}\n\
         # `listen` defaults to loopback. A public bind with [auth] mode = \"none\" lets\n\
         # anyone who can reach it publish telemetry signed with a freshly generated key.\n",
        crate::ingress::http::route_summary()
    );
    let mut out = String::with_capacity(rendered.len() + comment.len());
    let mut annotated = false;
    for line in rendered.split_inclusive('\n') {
        if !annotated && line.trim_end() == "[http]" {
            out.push_str(&comment);
            annotated = true;
        }
        out.push_str(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingress::http::INGRESS_PATH;

    #[test]
    fn generated_config_documents_ingress_route() {
        let rendered = toml::to_string_pretty(&Config::default()).unwrap();
        let annotated = annotate_http_section(&rendered);

        let header = annotated.find("\n[http]\n").expect("http header present");
        let comment = &annotated[..header];
        assert!(comment.contains(&format!("POST http://<[http].listen>{INGRESS_PATH}")));

        // Comments must not change the parsed configuration.
        let reparsed: Config = toml::from_str(&annotated).expect("annotated config parses");
        assert_eq!(reparsed.http.listen, Config::default().http.listen);
    }
}
