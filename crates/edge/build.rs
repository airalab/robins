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
//! Build script for the `edge` crate.
//!
//! Protobuf code generation follows the same pattern as `meshtastic/rust`: the
//! generated Rust module is committed to the repository and regeneration is an
//! explicit, developer-only step gated behind the `gen` feature. Normal builds
//! are a no-op here and simply `include!` the committed module, so consumers
//! never need `protoc` on their `PATH`.

#[cfg(not(feature = "gen"))]
fn main() {}

#[cfg(feature = "gen")]
fn main() -> std::io::Result<()> {
    // Vendored Connectivity Protocol sources and the committed output directory.
    let proto_dir = "src/protobufs/";
    let out_dir = "src/protocol/generated/";

    println!("cargo:rerun-if-changed={proto_dir}");
    println!("cargo:rerun-if-changed={out_dir}");

    // Ship a `protoc` binary so regeneration works without a system install.
    match protoc_bin_vendored::protoc_bin_path() {
        Ok(protoc_path) => {
            if std::env::var_os("PROTOC").is_some() {
                println!("Using PROTOC set in the environment.");
            } else {
                println!("Setting PROTOC to the protoc-bin-vendored binary.");
                std::env::set_var("PROTOC", protoc_path);
            }
        }
        Err(err) => {
            println!(
                "cargo:warning=protoc-bin-vendored unavailable, relying on system protoc: {err}"
            );
        }
    }

    // The gateway itself only needs the signed envelope: telemetry payloads are
    // opaque bytes forwarded through the pipeline untouched. The `edge codec`
    // CLI, however, also builds the complete `core.v1.Message` telemetry
    // payload (metadata + urban/insight device readings) that is placed inside
    // an envelope's `message` field before signing, so all vendored `.proto`
    // files are compiled. Collect them deterministically so regenerated output
    // is stable.
    let mut protos: Vec<_> = walkdir::WalkDir::new(proto_dir)
        .into_iter()
        .filter_map(Result::ok)
        .map(walkdir::DirEntry::into_path)
        .filter(|p| p.extension().is_some_and(|ext| ext == "proto"))
        .collect();
    protos.sort();

    std::fs::create_dir_all(out_dir)?;

    let mut config = prost_build::Config::new();
    config.out_dir(out_dir);
    config.compile_protos(&protos, &[proto_dir])?;

    // The committed output is checked by the repository's license header CI
    // job, so every regenerated `.rs` file needs the Apache-2.0 header
    // prepended (prost-build has no hook to inject it itself).
    const LICENSE_HEADER: &str = include_str!("../../.github/license-check/HEADER-APACHE2");
    for entry in std::fs::read_dir(out_dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "rs") {
            let body = std::fs::read_to_string(&path)?;
            std::fs::write(&path, format!("{LICENSE_HEADER}\n{body}"))?;
        }
    }

    Ok(())
}
