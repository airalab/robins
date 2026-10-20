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
//! Black-box tests of the `cps` profile commands and `cps status`.
//!
//! No live chain is needed: `status` talks to a tiny in-process JSON-RPC
//! WebSocket server, or to a closed port.

use assert_cmd::Command;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::path::PathBuf;
use tempfile::TempDir;
use tokio_tungstenite::tungstenite::Message;

/// Answer to one JSON-RPC request of the mock endpoint; `Err` is an RPC error.
fn answer(method: &str, params: &Value, with_health: bool) -> Result<Value, ()> {
    Ok(match method {
        "chain_getBlockHash" => json!("0x1234"),
        "state_getRuntimeVersion" => {
            json!({ "specName": "robonomics", "specVersion": 52, "implVersion": 3 })
        }
        "chain_getHeader" if params.as_array().is_none_or(|p| p.is_empty()) => {
            json!({ "number": "0xbc614e" })
        }
        "chain_getHeader" => json!({ "number": "0xbc6146" }),
        "chain_getFinalizedHead" => json!("0xabcd"),
        "system_chain" => json!("Robonomics"),
        "system_health" if with_health => {
            json!({ "isSyncing": false, "peers": 3, "shouldHavePeers": true })
        }
        _ => return Err(()),
    })
}

/// Start a mock RPC endpoint on an ephemeral port and return its `ws://` URL.
fn mock_rpc(with_health: bool) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                    while let Some(Ok(Message::Text(text))) = ws.next().await {
                        let request: Value = serde_json::from_str(&text).unwrap();
                        let method = request["method"].as_str().unwrap();
                        let reply = match answer(method, &request["params"], with_health) {
                            Ok(result) => {
                                json!({ "jsonrpc": "2.0", "id": request["id"], "result": result })
                            }
                            Err(()) => json!({
                                "jsonrpc": "2.0",
                                "id": request["id"],
                                "error": { "code": -32601, "message": "Method not found" },
                            }),
                        };
                        ws.send(Message::text(reply.to_string())).await.unwrap();
                    }
                });
            }
        });
    });
    url
}

/// A URL of a port nothing listens on.
fn closed_port_url() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    format!("ws://{}", listener.local_addr().unwrap())
}

/// Temporary home with an optional config file.
struct Env {
    dir: TempDir,
    config: PathBuf,
}

impl Env {
    fn new(config: &str) -> Self {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("cps.toml");
        std::fs::write(&config_path, config).unwrap();
        Self {
            dir,
            config: config_path,
        }
    }

    /// `cps` isolated from the real environment; reads this config via `--config`.
    fn cps(&self) -> Command {
        let mut cmd = Command::cargo_bin("cps").unwrap();
        cmd.env_clear()
            .env("HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.dir.path().join("xdg"))
            .arg("--config")
            .arg(&self.config);
        cmd
    }
}

const PROFILES: &str = r#"
default-profile = "local"

[profiles.local]
url = "ws://127.0.0.1:9944"

[profiles.production]
url = "wss://rpc.example.org"
output = "json"
wait = "best"
"#;

fn stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "not JSON ({e}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

#[test]
fn profile_list_marks_the_default() {
    let env = Env::new(PROFILES);
    let out = env.cps().args(["profile", "list"]).assert().success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(text.starts_with("NAME"), "{text}");
    assert!(text.contains("local") && text.contains("yes"), "{text}");
    assert!(text.contains("production"), "{text}");
}

#[test]
fn profile_list_json() {
    let env = Env::new(PROFILES);
    let out = env
        .cps()
        .args(["--output", "json", "profile", "list"])
        .assert()
        .success();
    let json = stdout_json(out.get_output());
    assert_eq!(json["default"], "local");
    assert_eq!(
        json["profiles"],
        json!([
            { "name": "local", "default": true },
            { "name": "production", "default": false },
        ])
    );
}

#[test]
fn profile_show_named_and_selected() {
    let env = Env::new(PROFILES);
    let out = env
        .cps()
        .args(["--output", "json", "profile", "show", "production"])
        .assert()
        .success();
    let json = stdout_json(out.get_output());
    assert_eq!(json["profile"], "production");
    assert_eq!(json["rpc"], "wss://rpc.example.org");
    assert_eq!(json["output"], "json");
    assert_eq!(json["wait"], "best");

    let out = env
        .cps()
        .args(["--output", "json", "profile", "show"])
        .assert()
        .success();
    assert_eq!(stdout_json(out.get_output())["profile"], "local");
}

#[test]
fn profile_show_without_name_applies_overrides_and_never_connects() {
    let env = Env::new(PROFILES);
    let out = env
        .cps()
        .env("CPS_PROFILE", "production")
        .args(["--ws-url", "ws://127.0.0.1:1", "profile", "show"])
        .assert()
        .success();
    let json = stdout_json(out.get_output());
    assert_eq!(json["profile"], "production");
    assert_eq!(json["rpc"], "ws://127.0.0.1:1");
}

#[test]
fn unknown_profile_lists_the_available_ones() {
    let env = Env::new(PROFILES);
    let out = env
        .cps()
        .args(["--profile", "prodction", "status"])
        .assert()
        .code(2);
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(
        stderr.contains("profile `prodction` does not exist"),
        "{stderr}"
    );
    assert!(stderr.contains("production"), "{stderr}");
}

#[test]
fn invalid_toml_and_missing_explicit_config_fail() {
    let env = Env::new("default-profile = [");
    env.cps().args(["profile", "list"]).assert().code(2);

    let mut cmd = Command::cargo_bin("cps").unwrap();
    cmd.env_clear()
        .args(["--config", "/nonexistent/cps.toml", "profile", "list"])
        .assert()
        .code(2);
}

#[test]
fn missing_default_config_is_not_an_error() {
    let dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("cps").unwrap();
    let out = cmd
        .env_clear()
        .env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", dir.path())
        .args(["--output", "json", "profile", "show"])
        .assert()
        .success();
    let json = stdout_json(out.get_output());
    assert_eq!(json["profile"], Value::Null);
    assert_eq!(json["rpc"], "ws://127.0.0.1:9944");
}

#[test]
fn status_reports_the_chain() {
    let url = mock_rpc(true);
    let env = Env::new(PROFILES);
    let out = env
        .cps()
        .args(["--ws-url", &url, "status"])
        .assert()
        .success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    for expected in [
        "local",
        &url,
        "Connected:",
        "yes",
        "Robonomics",
        "0x1234",
        "52",
        "#12345678",
        "#12345670",
        "no",
    ] {
        assert!(text.contains(expected), "missing `{expected}` in:\n{text}");
    }
}

#[test]
fn status_json_output_and_precedence() {
    let url = mock_rpc(true);
    let env = Env::new(PROFILES);
    // The profile points to an unreachable URL; the explicit option must win.
    let out = env
        .cps()
        .env("CPS_PROFILE", "production")
        .args(["--ws-url", &url, "--output", "json", "status"])
        .assert()
        .success();
    let json = stdout_json(out.get_output());
    assert_eq!(json["profile"], "production");
    assert_eq!(json["rpc"], url);
    assert_eq!(json["connected"], true);
    assert_eq!(json["config"], env.config.to_str().unwrap());
    assert_eq!(json["chain"]["name"], "Robonomics");
    assert_eq!(json["chain"]["genesis_hash"], "0x1234");
    assert_eq!(json["chain"]["spec_version"], 52);
    assert_eq!(json["chain"]["best"], 12345678);
    assert_eq!(json["chain"]["finalized"], 12345670);
    assert_eq!(json["chain"]["syncing"], false);
}

#[test]
fn status_without_sync_information_reports_unknown() {
    let url = mock_rpc(false);
    let env = Env::new(PROFILES);
    let out = env
        .cps()
        .args(["--ws-url", &url, "status"])
        .assert()
        .success();
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(
        text.contains("Syncing:") && text.contains("unknown"),
        "{text}"
    );

    let out = env
        .cps()
        .args(["--ws-url", &url, "--output", "json", "status"])
        .assert()
        .success();
    assert_eq!(
        stdout_json(out.get_output())["chain"]["syncing"],
        Value::Null
    );
}

#[test]
fn status_keeps_the_configuration_when_the_endpoint_is_down() {
    let url = closed_port_url();
    let env = Env::new(PROFILES);
    let out = env
        .cps()
        .args(["--ws-url", &url, "status"])
        .assert()
        .code(2);
    let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert!(text.contains("local") && text.contains(&url), "{text}");
    assert!(text.contains("no") && text.contains("Error:"), "{text}");

    let out = env
        .cps()
        .args(["--ws-url", &url, "--output", "json", "status"])
        .assert()
        .code(2);
    let json = stdout_json(out.get_output());
    assert_eq!(json["connected"], false);
    assert_eq!(json["rpc"], url);
    assert_eq!(json["profile"], "local");
    assert!(json["error"].as_str().is_some_and(|e| !e.is_empty()));
    assert!(json.get("chain").is_none());
}
