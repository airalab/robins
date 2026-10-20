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
//! `cps profile`: inspect the configured profiles. Never connects to the chain.

use crate::cli::{
    config::{display_path, Config, EffectiveConfig},
    context::Ctx,
    output::wide_fields,
};
use anyhow::Result;
use clap::Subcommand;
use serde_json::json;

/// `cps profile` subcommands.
#[derive(Subcommand)]
pub enum ProfileCommand {
    /// List the configured profiles
    List,
    /// Show the resolved settings of a profile (default: the selected profile)
    Show {
        /// Profile name; without it, the profile selected by --profile,
        /// CPS_PROFILE or `default-profile`, with all overrides applied
        name: Option<String>,
    },
}

/// Run a `cps profile` subcommand.
pub fn run(ctx: &Ctx, command: ProfileCommand) -> Result<()> {
    match command {
        ProfileCommand::List => list(ctx),
        ProfileCommand::Show { name } => show(ctx, name.as_deref()),
    }
}

fn list(ctx: &Ctx) -> Result<()> {
    let file = &ctx.file;
    let is_default = |name: &str| file.default_profile.as_deref() == Some(name);
    ctx.out.result(
        || human_list(file),
        json!({
            "default": file.default_profile,
            "profiles": file
                .profiles
                .keys()
                .map(|name| json!({ "name": name, "default": is_default(name) }))
                .collect::<Vec<_>>(),
        }),
    );
    Ok(())
}

/// `NAME  DEFAULT` table of the profiles.
fn human_list(file: &Config) -> String {
    if file.profiles.is_empty() {
        return "No profiles are configured.".into();
    }
    let width = file
        .profiles
        .keys()
        .map(String::len)
        .max()
        .unwrap_or(0)
        .max(4)
        + 2;
    let mut lines = vec![format!("{:<width$}DEFAULT", "NAME")];
    for name in file.profiles.keys() {
        let marker = if file.default_profile.as_deref() == Some(name) {
            "yes"
        } else {
            ""
        };
        lines.push(format!("{name:<width$}{marker}").trim_end().to_string());
    }
    lines.join("\n")
}

fn show(ctx: &Ctx, name: Option<&str>) -> Result<()> {
    let view = match name {
        Some(name) => ctx.resolved_profile(name)?,
        None => ctx.cfg.clone(),
    };
    ctx.out.result(|| human_show(&view), view.json());
    Ok(())
}

/// Aligned rows of the resolved profile settings.
fn human_show(view: &EffectiveConfig) -> String {
    wide_fields(&[
        (
            "Profile",
            view.profile
                .clone()
                .unwrap_or_else(|| "- (built-in defaults)".into()),
        ),
        ("Config", display_path(&view.config_path)),
        ("RPC", view.url.clone()),
        ("Output", view.output.to_string()),
        ("Wait", view.wait.to_string()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_marks_the_default_profile() {
        let file =
            Config::parse("default-profile = \"local\"\n[profiles.local]\n[profiles.production]\n")
                .unwrap();
        assert_eq!(
            human_list(&file),
            "NAME        DEFAULT\nlocal       yes\nproduction"
        );
    }
}
