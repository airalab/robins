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
//! Output rendering: results on stdout (human text or JSON), diagnostics on stderr.
//!
//! Keeping diagnostics off stdout leaves stdout clean for JSON, JSON Lines and
//! raw binary payloads that are redirected or piped.

use clap::ValueEnum;
use colored::*;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;
use serde_json::Value;

/// Format of command results.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    /// Human-readable text.
    #[default]
    Human,
    /// One JSON document per result; watches emit JSON Lines.
    Json,
}

/// Renders command results according to the selected [`OutputFormat`].
#[derive(Clone, Copy, Debug)]
pub struct Output {
    format: OutputFormat,
}

impl Output {
    /// Output in the given format.
    pub fn new(format: OutputFormat) -> Self {
        Self { format }
    }

    /// Whether results are machine-readable.
    pub fn is_json(&self) -> bool {
        self.format == OutputFormat::Json
    }

    /// Print a command result: `human()` as text, or `json` on a single line.
    pub fn result(&self, human: impl FnOnce() -> String, json: Value) {
        if self.is_json() {
            println!("{json}");
        } else {
            let text = human();
            if !text.is_empty() {
                println!("{text}");
            }
        }
    }

    /// Print one independently parseable event: `human()` as text, or `json` as
    /// one JSON Lines record.
    pub fn event(&self, human: impl FnOnce() -> String, json: Value) {
        self.result(human, json);
    }
}

/// Aligned `Label:  value` rows, as printed by `show` and `scope resolve`.
pub fn fields(rows: &[(&str, String)]) -> String {
    rows.iter()
        .map(|(label, value)| format!("{:<10} {value}", format!("{label}:")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Rows aligned on the longest label, for commands with long labels.
pub fn wide_fields(rows: &[(&str, String)]) -> String {
    let width = rows.iter().map(|(label, _)| label.len()).max().unwrap_or(0) + 1;
    rows.iter()
        .map(|(label, value)| format!("{:<width$} {value}", format!("{label}:")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Progress message on stderr.
pub fn progress(msg: &str) {
    eprintln!("{} {}", "[~]".cyan().bold(), msg.cyan());
}

/// Informational message on stderr.
pub fn info(msg: &str) {
    eprintln!("{} {}", "[i]".blue().bold(), msg.bright_blue());
}

/// Success message on stderr.
pub fn success(msg: &str) {
    eprintln!("{} {}", "[+]".green().bold(), msg.green());
}

/// Warning on stderr.
pub fn warn(msg: &str) {
    eprintln!("{} {}", "[!]".yellow().bold(), msg.yellow());
}

/// Spinner on stderr for a long-running operation.
pub fn spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
            .template("{spinner:.cyan.bold} {msg:.cyan}")
            .unwrap(),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(std::time::Duration::from_millis(100));
    pb
}

/// `0x`-prefixed lowercase hex.
pub fn hex0x(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_aligned() {
        let text = fields(&[("Node", "42".into()), ("Identity", "-".into())]);
        assert_eq!(text, "Node:      42\nIdentity:  -");
    }

    #[test]
    fn hex_is_prefixed() {
        assert_eq!(hex0x(&[0xab, 0x01]), "0xab01");
    }
}
