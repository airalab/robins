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

/// A deterministic 32-byte Ed25519 secret key, hex-encoded, for reproducibility.
const KEY_HEX: &str = "0707070707070707070707070707070707070707070707070707070707070707";

/// Write the fixture key to a temp file and return its path.
fn write_key() -> std::path::PathBuf {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("edge-it-key-{}.hex", std::process::id()));
    std::fs::write(&path, KEY_HEX).expect("write key");
    path
}

#[test]
fn version_prints_and_succeeds() {
    let (code, stdout, _) = run_edge(&["version"], b"");
    assert_eq!(code, 0);
    assert!(stdout.starts_with("edge "), "got: {stdout}");
}

#[test]
fn sign_verify_id_pipeline() {
    let key = write_key();
    let key_str = key.to_str().unwrap();

    // Sign a message, emitting the envelope as hex.
    let (code, envelope_hex, stderr) = run_edge(
        &[
            "key",
            "sign",
            "--key",
            key_str,
            "--timestamp",
            "1700000000000",
            "--nonce",
            "00112233445566778899aabbccddeeff",
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

    // Verify the freshly signed envelope: exit 0, "valid".
    let (code, stdout, _) = run_edge(
        &["key", "verify", "--input", "hex"],
        envelope_hex.as_bytes(),
    );
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "valid");

    // The dedup id is a stable 32-byte SHA-256 (64 hex chars).
    let (code, id_out, _) = run_edge(&["codec", "id", "--input", "hex"], envelope_hex.as_bytes());
    assert_eq!(code, 0);
    assert_eq!(id_out.trim().len(), 64);

    // A round-trip through JSON reproduces the exact wire bytes.
    let (code, json, _) = run_edge(
        &["codec", "decode", "--input", "hex", "--output", "json"],
        envelope_hex.as_bytes(),
    );
    assert_eq!(code, 0);
    let (code, reencoded, _) = run_edge(
        &["codec", "encode", "--input", "json", "--output", "hex"],
        json.as_bytes(),
    );
    assert_eq!(code, 0);
    assert_eq!(reencoded.trim(), envelope_hex);

    let _ = std::fs::remove_file(&key);
}

#[test]
fn tampered_envelope_fails_verification() {
    let key = write_key();
    let key_str = key.to_str().unwrap();

    let (_, envelope_hex, _) = run_edge(
        &[
            "key", "sign", "--key", key_str, "--input", "binary", "--output", "hex",
        ],
        b"payload",
    );
    let mut bytes = envelope_hex.trim().to_string();
    // Flip the final hex nibble to corrupt the signature.
    let last = bytes.pop().unwrap();
    bytes.push(if last == '0' { '1' } else { '0' });

    let (code, _, stderr) = run_edge(&["key", "verify", "--input", "hex"], bytes.as_bytes());
    assert_eq!(
        code, 1,
        "expected invalid-signature exit code; stderr: {stderr}"
    );

    let _ = std::fs::remove_file(&key);
}

#[test]
fn gw_and_node_report_unimplemented() {
    let (code, _, stderr) = run_edge(&["gw"], b"");
    assert_eq!(code, 1);
    assert!(stderr.contains("not yet implemented"), "stderr: {stderr}");

    let (code, _, stderr) = run_edge(&["node"], b"");
    assert_eq!(code, 1);
    assert!(stderr.contains("not yet implemented"), "stderr: {stderr}");
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
    let key = write_key();
    let key_str = key.to_str().unwrap();

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
            "key", "sign", "--key", key_str, "--input", "binary", "--output", "hex",
        ],
        b"telemetry",
    );
    let (code, verify_json, _) = run_edge(
        &["key", "verify", "--input", "hex", "--output", "json"],
        envelope_hex.trim().as_bytes(),
    );
    assert_eq!(code, 0);
    let verified: serde_json::Value = serde_json::from_str(&verify_json).expect("valid json");
    assert_eq!(verified["sensor_id"].as_str().unwrap(), ss58);

    let _ = std::fs::remove_file(&key);
}
