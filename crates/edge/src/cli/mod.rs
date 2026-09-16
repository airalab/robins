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
//! Command-line interface for the `edge` gateway.
//!
//! The CLI is deliberately thin: every command reuses the canonical library
//! logic ([`crate::protocol`], [`crate::config`]) and never re-implements crypto
//! or parsing. Following Unix conventions, pipeline **data** goes to `stdout`
//! while **logs and diagnostics** go to `stderr`, and secret material is never
//! printed.
//!
//! ## Exit codes
//!
//! Commands map failures onto a stable status contract so they compose in
//! scripts:
//!
//! - `0` — success.
//! - `1` — runtime error (I/O, unimplemented mode, unexpected failure).
//! - `2` — CLI usage or configuration error.
//! - `3` — protocol / input error (malformed or invalid envelope).

mod config;
mod envelope;
mod format;
mod key;
mod message;

use clap::{Parser, Subcommand};
use std::process::ExitCode;

/// A CLI failure carrying the process exit code to surface to the shell.
#[derive(Debug)]
pub(crate) struct CliError {
    /// Process exit code (see the module-level contract).
    code: u8,
    /// Human-readable message, written to `stderr`.
    message: String,
}

impl CliError {
    /// Runtime error (exit code `1`).
    pub(crate) fn runtime(message: impl Into<String>) -> Self {
        Self {
            code: 1,
            message: message.into(),
        }
    }

    /// CLI usage or configuration error (exit code `2`).
    pub(crate) fn usage(message: impl Into<String>) -> Self {
        Self {
            code: 2,
            message: message.into(),
        }
    }

    /// Explicit code with message (used to relay protocol exit codes).
    pub(crate) fn with_code(code: u8, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Result alias for CLI command handlers.
pub(crate) type CliResult = Result<(), CliError>;

/// Robonomics Edge Gateway — Connectivity Protocol ingress for edge devices.
#[derive(Debug, Parser)]
#[command(name = "edge", version, about, long_about = None)]
#[command(before_help = r#"
╔══════════════════════════════════════════════════════╗
║                                                      ║
║          ███████╗██████╗  ██████╗ ███████╗           ║
║          ██╔════╝██╔══██╗██╔════╝ ██╔════╝           ║
║          █████╗  ██║  ██║██║  ███╗█████╗             ║
║          ██╔══╝  ██║  ██║██║   ██║██╔══╝             ║
║          ███████╗██████╔╝╚██████╔╝███████╗           ║
║          ╚══════╝╚═════╝  ╚═════╝ ╚══════╝           ║
║                                                      ║
║          Edge Gateway - Robonomics Network           ║
║                                                      ║
╚══════════════════════════════════════════════════════╝
"#)]
struct Cli {
    /// Log verbosity (`error`, `warn`, `info`, `debug`, `trace`).
    #[arg(long, global = true, default_value = "info", env = "EDGE_LOG_LEVEL")]
    log_level: String,

    /// Disable ANSI colour in log output.
    #[arg(long, global = true)]
    no_color: bool,

    /// Suppress non-error logs (equivalent to `--log-level error`).
    #[arg(long, short, global = true)]
    quiet: bool,

    #[command(subcommand)]
    command: Command,
}

/// Top-level command tree.
#[derive(Debug, Subcommand)]
enum Command {
    /// Run the long-running gateway daemon [ALIAS: g].
    #[command(alias = "g")]
    Gateway {
        /// Path to the gateway TOML configuration file.
        #[arg(short, long, default_value = crate::config::DEFAULT_CONFIG_PATH)]
        config: std::path::PathBuf,
    },
    /// Produce signed envelopes from local sources !!! not yet implemented !!!.
    #[command(alias = "s")]
    Sensor {
        /// Captured arguments (parsing deferred until implemented).
        #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// Encode or decode a `SignedEnvelope` [ALIAS: e].
    #[command(alias = "e")]
    Envelope(envelope::EnvelopeArgs),
    /// Encode or decode a `Message` telemetry payload [ALIAS: m].
    #[command(alias = "m")]
    Message(message::MessageArgs),
    /// Generate or inspect sensor identity keys [ALIAS: k].
    #[command(subcommand, alias = "k")]
    Key(key::KeyCommand),
    /// Validate and print gateway configuration [ALIAS: c].
    #[command(subcommand, alias = "c")]
    Config(config::ConfigCommand),
}

/// Parse arguments, initialise logging, dispatch, and return the exit code.
///
/// This is the single entry point used by `main`; it owns the mapping from a
/// [`CliError`] to the process [`ExitCode`].
pub fn run() -> ExitCode {
    let cli = Cli::parse();
    init_logging(&cli);
    if cli.no_color {
        // Also disables `colored` output on stdout (used by `edge envelope`
        // and `edge message`'s human-readable `text` rendering), not just
        // `env_logger`'s stderr output.
        colored::control::set_override(false);
    }

    let result = match cli.command {
        Command::Gateway { config } => run_gateway(config),
        Command::Sensor { .. } => Err(CliError::runtime(
            "`edge sensor` is not yet implemented in this build",
        )),
        Command::Envelope(args) => envelope::run(args),
        Command::Message(args) => message::run(args),
        Command::Key(cmd) => key::run(cmd),
        Command::Config(cmd) => config::run(cmd),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {}", err.message);
            ExitCode::from(err.code)
        }
    }
}

/// Load `config`, build a multi-threaded Tokio runtime, and run the gateway
/// until it is asked to shut down.
///
/// Configuration errors map to exit code `2` (usage), while runtime failures
/// (binding sockets, subsystem startup) map to exit code `1`.
fn run_gateway(config: std::path::PathBuf) -> CliResult {
    let config =
        crate::config::Config::from_file(&config).map_err(|e| CliError::usage(e.to_string()))?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::runtime(format!("failed to start async runtime: {e}")))?;

    runtime
        .block_on(crate::app::run(config))
        .map_err(|e| CliError::runtime(format!("{e:#}")))
}

/// Configure `env_logger` from the global flags. Logs are written to `stderr`
/// so they never contaminate the data written to `stdout`.
fn init_logging(cli: &Cli) {
    let level = if cli.quiet { "error" } else { &cli.log_level };
    env_logger::Builder::new()
        .parse_filters(level)
        .format_timestamp_millis()
        .write_style(if cli.no_color {
            env_logger::WriteStyle::Never
        } else {
            env_logger::WriteStyle::Auto
        })
        .target(env_logger::Target::Stderr)
        .try_init()
        .ok();
}
