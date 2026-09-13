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
//! Black-box integration tests driving the `edge` binary through pipelines.

use std::io::Write;
use std::process::{Command, Stdio};

/// Path to the compiled `edge` binary provided by Cargo to integration tests.
const EDGE_BIN: &str = env!("CARGO_BIN_EXE_edge");

/// Run `edge <args>` feeding `stdin`, returning `(exit_code, stdout, stderr)`.
fn run_edge(args: &[&str], stdin: &[u8]) -> (i32, String, String) {
    let mut child = Command::new(EDGE_BIN)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn edge");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin)
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait edge");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// A deterministic 32-byte Ed25519 secret key, `0x`-prefixed hex, for
/// reproducibility (raw hex seeds must carry the `0x` prefix; see
/// [`SensorIdentity::from_suri`](edge::protocol::SensorIdentity::from_suri)).
const KEY_HEX: &str = "0x0707070707070707070707070707070707070707070707070707070707070707";

#[test]
fn version_prints_and_succeeds() {
    let (code, stdout, _) = run_edge(&["version"], b"");
    assert_eq!(code, 0);
    assert!(stdout.starts_with("edge "), "got: {stdout}");
}

#[test]
fn sign_verify_id_pipeline() {
    // Sign a message, emitting the envelope as hex.
    let (code, envelope_hex, stderr) = run_edge(
        &[
            "envelope",
            "--sign",
            KEY_HEX,
            "--timestamp",
            "1700000000000",
            "--nonce",
            "0x00112233445566778899aabbccddeeff",
            "--input",
            "binary",
            "--output",
            "hex",
        ],
        b"hello world",
    );
    assert_eq!(code, 0, "sign failed: {stderr}");
    let envelope_hex = envelope_hex.trim().to_string();
    assert!(!envelope_hex.is_empty());

    // Verify the freshly signed envelope: exit 0, diagnostic on stderr.
    let (code, _, stderr) = run_edge(
        &["envelope", "--verify", "--input", "hex", "--output", "hex"],
        envelope_hex.as_bytes(),
    );
    assert_eq!(code, 0);
    assert!(stderr.contains("signature: valid"), "stderr: {stderr}");

    // The dedup id is a stable 32-byte SHA-256 (64 hex chars), `0x`-prefixed,
    // printed on stderr for non-JSON decode output.
    let (code, _, id_stderr) = run_edge(&["envelope", "--input", "hex"], envelope_hex.as_bytes());
    assert_eq!(code, 0);
    let id_line = id_stderr
        .lines()
        .find(|l| l.starts_with("message_id: "))
        .expect("message_id line");
    let message_id = id_line.trim_start_matches("message_id: ");
    assert!(message_id.starts_with("0x"), "{message_id}");
    assert_eq!(message_id.trim_start_matches("0x").len(), 64);

    // A round-trip through JSON reproduces the exact wire bytes.
    let (code, json, _) = run_edge(
        &["envelope", "--input", "hex", "--output", "json"],
        envelope_hex.as_bytes(),
    );
    assert_eq!(code, 0);
    let (code, reencoded, _) = run_edge(
        &["envelope", "--input", "json", "--output", "hex"],
        json.as_bytes(),
    );
    assert_eq!(code, 0);
    assert_eq!(reencoded.trim(), envelope_hex);
}

#[test]
fn tampered_envelope_fails_verification() {
    let (_, envelope_hex, _) = run_edge(
        &[
            "envelope", "--sign", KEY_HEX, "--input", "binary", "--output", "hex",
        ],
        b"payload",
    );
    let mut bytes = envelope_hex.trim().to_string();
    // Flip the final hex nibble to corrupt the signature.
    let last = bytes.pop().unwrap();
    bytes.push(if last == '0' { '1' } else { '0' });

    let (code, _, stderr) = run_edge(
        &["envelope", "--verify", "--input", "hex"],
        bytes.as_bytes(),
    );
    assert_eq!(
        code, 1,
        "expected invalid-signature exit code; stderr: {stderr}"
    );
}

#[test]
fn sensor_reports_unimplemented() {
    let (code, _, stderr) = run_edge(&["sensor"], b"");
    assert_eq!(code, 1);
    assert!(stderr.contains("not yet implemented"), "stderr: {stderr}");
}

#[test]
fn gw_missing_config_is_usage_error() {
    // With no readable config at the (non-existent) path, `gw` fails fast with a
    // usage error (exit 2) rather than starting the daemon.
    let (code, _, stderr) = run_edge(&["gw", "--config", "/nonexistent/edge/gateway.toml"], b"");
    assert_eq!(code, 2, "stderr: {stderr}");
}

#[test]
fn key_generate_reports_identity() {
    let (code, stdout, _) = run_edge(&["key", "generate"], b"");
    assert_eq!(code, 0);
    assert!(stdout.contains("Secret seed:"), "got: {stdout}");
    assert!(stdout.contains("SS58 Address:"), "got: {stdout}");

    let (code, json, _) = run_edge(&["key", "generate", "--output", "json"], b"");
    assert_eq!(code, 0);
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    assert!(value["ss58Address"].is_string());
    assert!(value["publicKey"].as_str().unwrap().starts_with("0x"));
}

#[test]
fn key_inspect_matches_generate_and_sign() {
    // Inspect the fixture key URI (hex secret seed): JSON exposes a stable SS58.
    let (code, json, stderr) = run_edge(&["key", "inspect", KEY_HEX, "--output", "json"], b"");
    assert_eq!(code, 0, "inspect failed: {stderr}");
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    let ss58 = value["ss58Address"].as_str().expect("ss58").to_string();
    assert!(!ss58.is_empty());
    assert!(
        value.get("secretSeed").is_some(),
        "secret expected for seed URI"
    );

    // Inspecting the public SS58 URI must omit any secret material.
    let (code, pub_json, _) = run_edge(&["key", "inspect", &ss58, "--output", "json"], b"");
    assert_eq!(code, 0);
    let pub_value: serde_json::Value = serde_json::from_str(&pub_json).expect("valid json");
    assert!(
        pub_value.get("secretSeed").is_none(),
        "public inspect leaked secret"
    );
    assert_eq!(pub_value["ss58Address"].as_str().unwrap(), ss58);

    // The same key signing an envelope must verify to the same sensor_id.
    let (_, envelope_hex, _) = run_edge(
        &[
            "envelope", "--sign", KEY_HEX, "--input", "binary", "--output", "hex",
        ],
        b"telemetry",
    );
    let (code, verify_json, _) = run_edge(
        &["envelope", "--verify", "--input", "hex", "--output", "json"],
        envelope_hex.trim().as_bytes(),
    );
    assert_eq!(code, 0);
    let verified: serde_json::Value = serde_json::from_str(&verify_json).expect("valid json");
    let sensor_id_hex = verified["sensor_id"].as_str().unwrap().to_string();
    let sensor_id = edge::protocol::SensorId::from_hex(&sensor_id_hex).expect("valid sensor_id");
    assert_eq!(sensor_id.to_ss58(), ss58);
}

#[test]
fn config_generate_produces_valid_config() {
    // Generating to stdout yields a TOML document that `config check` accepts.
    let (code, stdout, _) = run_edge(&["config", "generate"], b"");
    assert_eq!(code, 0);
    assert!(stdout.contains("[http]"), "got: {stdout}");
    assert!(stdout.contains("[pubsub]"), "got: {stdout}");

    let dir = std::env::temp_dir();
    let path = dir.join(format!("edge-it-gen-{}.toml", std::process::id()));
    let path_str = path.to_str().unwrap();

    let (code, _, stderr) = run_edge(&["config", "generate", "--output", path_str], b"");
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stderr.contains("wrote default configuration"),
        "stderr: {stderr}"
    );

    let (code, check_out, _) = run_edge(&["config", "check", "--config", path_str], b"");
    assert_eq!(code, 0);
    assert!(check_out.contains("valid"), "got: {check_out}");

    // Writing over an existing file requires --force.
    let (code, _, stderr) = run_edge(&["config", "generate", "--output", path_str], b"");
    assert_eq!(code, 2, "stderr: {stderr}");
    assert!(stderr.contains("already exists"), "stderr: {stderr}");

    let (code, _, _) = run_edge(
        &["config", "generate", "--output", path_str, "--force"],
        b"",
    );
    assert_eq!(code, 0);

    let _ = std::fs::remove_file(&path);
}
