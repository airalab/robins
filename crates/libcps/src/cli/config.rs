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
//! CLI configuration: the TOML config file, named profiles and the resolution of
//! the effective settings.
//!
//! Every setting is resolved here, once, with one fixed precedence (highest
//! first): command-line option, environment variable, selected profile,
//! built-in default. Commands consume the resulting [`EffectiveConfig`] and never
//! look at flags, the environment or the config file themselves.
//!
//! Resolution is a pure function of its inputs ([`CliSettings`], an environment
//! lookup and the config file path), so it is testable without a network.
//! Profiles hold CLI settings only; secret material is rejected by the parser
//! (unknown keys are errors) and must stay outside the config file.

use super::output::OutputFormat;
use anyhow::{anyhow, bail, Context, Result};
use clap::ValueEnum;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
};

/// Built-in RPC endpoint: the default listener of `cps light-client`.
pub const DEFAULT_URL: &str = "ws://127.0.0.1:9944";

/// Environment variable selecting the config file.
pub const ENV_CONFIG: &str = "CPS_CONFIG";
/// Environment variable selecting the profile.
pub const ENV_PROFILE: &str = "CPS_PROFILE";
/// Environment variable with the RPC endpoint.
pub const ENV_URL: &str = "ROBONOMICS_WS_URL";
/// Environment variable with the output format.
pub const ENV_OUTPUT: &str = "CPS_OUTPUT";
/// Environment variable with the transaction wait policy.
pub const ENV_WAIT: &str = "CPS_WAIT";

/// How long a transaction is awaited before the command reports its outcome.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WaitPolicy {
    /// Until the transaction is included in a best (not yet final) block.
    Best,
    /// Until the block with the transaction is finalized.
    #[default]
    Finalized,
}

/// Lowercase name of a clap value enum, as written in the config file.
fn value_name<T: ValueEnum>(value: &T) -> String {
    value
        .to_possible_value()
        .map(|v| v.get_name().to_string())
        .unwrap_or_default()
}

impl fmt::Display for WaitPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&value_name(self))
    }
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&value_name(self))
    }
}

/// One named set of CLI settings; absent keys fall through to the built-in defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Profile {
    /// WebSocket RPC endpoint.
    pub url: Option<String>,
    /// Result format.
    pub output: Option<OutputFormat>,
    /// Transaction wait policy.
    pub wait: Option<WaitPolicy>,
}

/// Contents of the config file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Config {
    /// Profile used when none is selected explicitly.
    pub default_profile: Option<String>,
    /// Named profiles.
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
}

impl Config {
    /// Parse and validate TOML text.
    pub fn parse(text: &str) -> Result<Self> {
        let config: Config = toml::from_str(text)?;
        for (name, profile) in &config.profiles {
            if let Some(url) = &profile.url {
                validate_url(url).with_context(|| format!("invalid option in profile `{name}`"))?;
            }
        }
        Ok(config)
    }

    /// The named profile, or an error listing the available ones.
    pub fn profile(&self, name: &str) -> Result<&Profile> {
        self.profiles.get(name).ok_or_else(|| {
            let mut message = format!("profile `{name}` does not exist");
            if self.profiles.is_empty() {
                message.push_str("\n\nNo profiles are configured.");
            } else {
                message.push_str("\n\nAvailable profiles:");
                for available in self.profiles.keys() {
                    message.push_str(&format!("\n  {available}"));
                }
            }
            anyhow!(message)
        })
    }
}

/// Settings given on the command line; `None` means "not given".
#[derive(Clone, Debug, Default)]
pub struct CliSettings {
    /// `--config`.
    pub config: Option<PathBuf>,
    /// `--profile`.
    pub profile: Option<String>,
    /// `--ws-url`.
    pub url: Option<String>,
    /// `--output`.
    pub output: Option<OutputFormat>,
    /// `--wait`.
    pub wait: Option<WaitPolicy>,
}

/// The settings every command runs with, after applying the precedence rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveConfig {
    /// Selected profile, if any.
    pub profile: Option<String>,
    /// Config file that was read; `None` when no config file exists.
    pub config_path: Option<PathBuf>,
    /// WebSocket RPC endpoint.
    pub url: String,
    /// Result format.
    pub output: OutputFormat,
    /// Transaction wait policy.
    pub wait: WaitPolicy,
}

impl EffectiveConfig {
    /// Settings of `profile` over the built-in defaults, without CLI or environment.
    pub fn of_profile(name: &str, profile: &Profile, config_path: Option<PathBuf>) -> Self {
        Self {
            profile: Some(name.to_string()),
            config_path,
            url: profile.url.clone().unwrap_or_else(|| DEFAULT_URL.into()),
            output: profile.output.unwrap_or_default(),
            wait: profile.wait.unwrap_or_default(),
        }
    }

    /// Machine-readable form, shared by `profile show` and `status`.
    pub fn json(&self) -> Value {
        json!({
            "profile": self.profile,
            "config": self.config_path,
            "rpc": self.url,
            "output": self.output.to_string(),
            "wait": self.wait.to_string(),
        })
    }
}

/// The loaded config file together with the settings resolved from it.
#[derive(Clone, Debug)]
pub struct Resolved {
    /// Parsed config file (empty if there is none).
    pub file: Config,
    /// Effective settings of this invocation.
    pub effective: EffectiveConfig,
}

/// Platform default config file, e.g. `~/.config/cps/config.toml` on Linux.
pub fn default_config_path(env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let non_empty = |name: &str| env(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = if cfg!(windows) {
        non_empty("APPDATA")?
    } else if cfg!(target_os = "macos") {
        non_empty("HOME")?.join("Library/Application Support")
    } else {
        non_empty("XDG_CONFIG_HOME").or_else(|| non_empty("HOME").map(|h| h.join(".config")))?
    };
    Some(base.join("cps").join("config.toml"))
}

/// Read the config file. Returns the parsed file and the path it was read from;
/// a missing file is an error only if it was requested explicitly.
fn load(path: Option<(PathBuf, bool)>) -> Result<(Config, Option<PathBuf>)> {
    let Some((path, explicit)) = path else {
        return Ok((Config::default(), None));
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let config = Config::parse(&text)
                .with_context(|| format!("cannot load config file {}", path.display()))?;
            Ok((config, Some(path)))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !explicit => {
            Ok((Config::default(), None))
        }
        Err(e) => bail!("cannot read config file {}: {e}", path.display()),
    }
}

/// Parse an environment value of a clap value enum.
fn env_enum<T: ValueEnum>(env: &dyn Fn(&str) -> Option<String>, name: &str) -> Result<Option<T>> {
    env(name)
        .filter(|v| !v.is_empty())
        .map(|v| T::from_str(&v, true).map_err(|e| anyhow!("invalid value of {name}: `{v}` ({e})")))
        .transpose()
}

/// Check that `url` is a WebSocket endpoint (`ws://` or `wss://`).
pub fn validate_url(url: &str) -> Result<()> {
    let rest = url
        .strip_prefix("ws://")
        .or_else(|| url.strip_prefix("wss://"));
    match rest {
        Some(rest)
            if !rest.is_empty() && !rest.starts_with('/') && !url.contains(char::is_whitespace) =>
        {
            Ok(())
        }
        _ => bail!("invalid RPC URL `{url}`: expected ws://HOST[:PORT] or wss://HOST[:PORT]"),
    }
}

/// Resolve the effective configuration.
///
/// `env` looks up environment variables; `default_path` is the platform default
/// config file location.
pub fn resolve(
    cli: CliSettings,
    env: &dyn Fn(&str) -> Option<String>,
    default_path: Option<PathBuf>,
) -> Result<Resolved> {
    let text = |name: &str| env(name).filter(|v| !v.is_empty());

    let config_path = cli
        .config
        .map(|p| (p, true))
        .or_else(|| text(ENV_CONFIG).map(|p| (PathBuf::from(p), true)))
        .or_else(|| default_path.map(|p| (p, false)));
    let (file, config_path) = load(config_path)?;

    let profile_name = cli
        .profile
        .or_else(|| text(ENV_PROFILE))
        .or_else(|| file.default_profile.clone());
    let profile = match &profile_name {
        Some(name) => file.profile(name)?.clone(),
        None => Profile::default(),
    };

    let url = cli
        .url
        .or_else(|| text(ENV_URL))
        .or(profile.url)
        .unwrap_or_else(|| DEFAULT_URL.into());
    validate_url(&url)?;

    let output = match cli.output {
        Some(output) => output,
        None => env_enum(env, ENV_OUTPUT)?
            .or(profile.output)
            .unwrap_or_default(),
    };
    let wait = match cli.wait {
        Some(wait) => wait,
        None => env_enum(env, ENV_WAIT)?
            .or(profile.wait)
            .unwrap_or_default(),
    };

    Ok(Resolved {
        file,
        effective: EffectiveConfig {
            profile: profile_name,
            config_path,
            url,
            output,
            wait,
        },
    })
}

/// Resolve from the process environment and the platform default config path.
pub fn resolve_from_env(cli: CliSettings) -> Result<Resolved> {
    let env = |name: &str| std::env::var(name).ok();
    resolve(cli, &env, default_config_path(&env))
}

/// Path of the config file, for display.
pub fn display_path(path: &Option<PathBuf>) -> String {
    path.as_deref()
        .map(Path::display)
        .map(|p| p.to_string())
        .unwrap_or_else(|| "-".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

    const FILE: &str = r#"
default-profile = "local"

[profiles.local]
url = "ws://127.0.0.1:9944"
output = "human"
wait = "finalized"

[profiles.dev]
url = "wss://dev.example.org"
output = "json"
wait = "best"

[profiles.production]
url = "wss://rpc.example.org"
"#;

    fn env_of<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| map.get(name).cloned()
    }

    fn write(dir: &TempDir, text: &str) -> PathBuf {
        let path = dir.path().join("config.toml");
        std::fs::write(&path, text).unwrap();
        path
    }

    fn resolve_with(
        cli: CliSettings,
        env: &[(&str, &str)],
        default_path: Option<PathBuf>,
    ) -> Result<EffectiveConfig> {
        resolve(cli, &env_of(env), default_path).map(|r| r.effective)
    }

    #[test]
    fn built_in_defaults_without_a_config_file() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("absent.toml");
        let cfg = resolve_with(CliSettings::default(), &[], Some(missing)).unwrap();
        assert_eq!(cfg.profile, None);
        assert_eq!(cfg.config_path, None);
        assert_eq!(cfg.url, DEFAULT_URL);
        assert_eq!(cfg.output, OutputFormat::Human);
        assert_eq!(cfg.wait, WaitPolicy::Finalized);
    }

    #[test]
    fn missing_default_config_path_is_not_an_error() {
        let cfg = resolve_with(CliSettings::default(), &[], None).unwrap();
        assert_eq!(cfg.url, DEFAULT_URL);
    }

    #[test]
    fn default_profile_is_selected() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, FILE);
        let cfg = resolve_with(CliSettings::default(), &[], Some(path.clone())).unwrap();
        assert_eq!(cfg.profile.as_deref(), Some("local"));
        assert_eq!(cfg.config_path, Some(path));
    }

    #[test]
    fn profile_flag_selects_a_profile() {
        let dir = TempDir::new().unwrap();
        let cli = CliSettings {
            profile: Some("dev".into()),
            ..Default::default()
        };
        let cfg = resolve_with(cli, &[], Some(write(&dir, FILE))).unwrap();
        assert_eq!(cfg.url, "wss://dev.example.org");
        assert_eq!(cfg.output, OutputFormat::Json);
        assert_eq!(cfg.wait, WaitPolicy::Best);
    }

    #[test]
    fn profile_env_selects_a_profile_and_flag_wins() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, FILE);
        let env = [(ENV_PROFILE, "production")];
        let cfg = resolve_with(CliSettings::default(), &env, Some(path.clone())).unwrap();
        assert_eq!(cfg.profile.as_deref(), Some("production"));
        let cli = CliSettings {
            profile: Some("dev".into()),
            ..Default::default()
        };
        let cfg = resolve_with(cli, &env, Some(path)).unwrap();
        assert_eq!(cfg.profile.as_deref(), Some("dev"));
    }

    #[test]
    fn cli_overrides_profile() {
        let dir = TempDir::new().unwrap();
        let cli = CliSettings {
            profile: Some("production".into()),
            url: Some("ws://127.0.0.1:9955".into()),
            output: Some(OutputFormat::Human),
            wait: Some(WaitPolicy::Best),
            ..Default::default()
        };
        let cfg = resolve_with(cli, &[], Some(write(&dir, FILE))).unwrap();
        assert_eq!(cfg.profile.as_deref(), Some("production"));
        assert_eq!(cfg.url, "ws://127.0.0.1:9955");
        assert_eq!(cfg.output, OutputFormat::Human);
        assert_eq!(cfg.wait, WaitPolicy::Best);
    }

    #[test]
    fn env_overrides_profile() {
        let dir = TempDir::new().unwrap();
        let cli = CliSettings {
            profile: Some("dev".into()),
            ..Default::default()
        };
        let env = [
            (ENV_URL, "ws://10.0.0.1:9944"),
            (ENV_OUTPUT, "human"),
            (ENV_WAIT, "finalized"),
        ];
        let cfg = resolve_with(cli, &env, Some(write(&dir, FILE))).unwrap();
        assert_eq!(cfg.url, "ws://10.0.0.1:9944");
        assert_eq!(cfg.output, OutputFormat::Human);
        assert_eq!(cfg.wait, WaitPolicy::Finalized);
    }

    #[test]
    fn cli_overrides_env() {
        let cli = CliSettings {
            url: Some("ws://127.0.0.1:1".into()),
            output: Some(OutputFormat::Json),
            wait: Some(WaitPolicy::Best),
            ..Default::default()
        };
        let env = [
            (ENV_URL, "ws://10.0.0.1:9944"),
            (ENV_OUTPUT, "human"),
            (ENV_WAIT, "finalized"),
        ];
        let cfg = resolve_with(cli, &env, None).unwrap();
        assert_eq!(cfg.url, "ws://127.0.0.1:1");
        assert_eq!(cfg.output, OutputFormat::Json);
        assert_eq!(cfg.wait, WaitPolicy::Best);
    }

    #[test]
    fn missing_explicit_config_is_an_error() {
        let dir = TempDir::new().unwrap();
        let cli = CliSettings {
            config: Some(dir.path().join("missing.toml")),
            ..Default::default()
        };
        assert!(resolve_with(cli, &[], None).is_err());
        let missing = dir.path().join("missing.toml");
        let env = [(ENV_CONFIG, missing.to_str().unwrap())];
        assert!(resolve_with(CliSettings::default(), &env, None).is_err());
    }

    #[test]
    fn config_flag_wins_over_env() {
        let dir = TempDir::new().unwrap();
        let good = write(&dir, FILE);
        let env = [(ENV_CONFIG, "/nonexistent/cps.toml")];
        let cli = CliSettings {
            config: Some(good.clone()),
            ..Default::default()
        };
        let cfg = resolve_with(cli, &env, None).unwrap();
        assert_eq!(cfg.config_path, Some(good));
    }

    #[test]
    fn unknown_profile_lists_the_available_ones() {
        let dir = TempDir::new().unwrap();
        let cli = CliSettings {
            profile: Some("prodction".into()),
            ..Default::default()
        };
        let error = resolve_with(cli, &[], Some(write(&dir, FILE)))
            .unwrap_err()
            .to_string();
        assert!(error.contains("profile `prodction` does not exist"));
        for name in ["dev", "local", "production"] {
            assert!(error.contains(name), "{error}");
        }
    }

    #[test]
    fn unknown_default_profile_is_an_error() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "default-profile = \"gone\"\n");
        assert!(resolve_with(CliSettings::default(), &[], Some(path)).is_err());
    }

    #[test]
    fn invalid_toml_is_an_error() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "default-profile = [");
        let error = resolve_with(CliSettings::default(), &[], Some(path)).unwrap_err();
        assert!(format!("{error:#}").contains("cannot load config file"));
    }

    #[test]
    fn invalid_profile_options_are_errors() {
        for bad in [
            "[profiles.a]\nwait = \"never\"\n",
            "[profiles.a]\noutput = \"xml\"\n",
            "[profiles.a]\nurl = \"http://example.org\"\n",
            "[profiles.a]\nsuri = \"//Alice\"\n",
        ] {
            assert!(Config::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn invalid_env_and_cli_urls_are_errors() {
        let env = [(ENV_URL, "http://example.org")];
        assert!(resolve_with(CliSettings::default(), &env, None).is_err());
        let env = [(ENV_OUTPUT, "xml")];
        assert!(resolve_with(CliSettings::default(), &env, None).is_err());
        let cli = CliSettings {
            url: Some("localhost:9944".into()),
            ..Default::default()
        };
        assert!(resolve_with(cli, &[], None).is_err());
    }

    #[test]
    fn default_path_follows_xdg_then_home() {
        let xdg = env_of(&[("XDG_CONFIG_HOME", "/xdg"), ("HOME", "/home/u")]);
        let home = env_of(&[("HOME", "/home/u")]);
        if cfg!(all(unix, not(target_os = "macos"))) {
            assert_eq!(
                default_config_path(&xdg),
                Some(PathBuf::from("/xdg/cps/config.toml"))
            );
            assert_eq!(
                default_config_path(&home),
                Some(PathBuf::from("/home/u/.config/cps/config.toml"))
            );
        }
        assert_eq!(default_config_path(&env_of(&[])), None);
    }
}
