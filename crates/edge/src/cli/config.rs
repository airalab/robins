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
}

/// Dispatch a `config` subcommand.
pub(crate) fn run(command: ConfigCommand) -> CliResult {
    match command {
        ConfigCommand::Check { config } => check(config),
        ConfigCommand::Print { config } => print(config),
    }
}

/// Load + validate the file, mapping any error to exit code `2`.
fn load(path: PathBuf) -> Result<Config, CliError> {
    Config::from_file(&path).map_err(|e| CliError::usage(e.to_string()))
}

/// Implements `edge config check`.
fn check(path: PathBuf) -> CliResult {
    let _ = load(path)?;
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
