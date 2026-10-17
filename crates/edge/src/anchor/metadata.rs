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
//! Desired CPS metadata, derived from the batching configuration.
//!
//! Payloads carry no framing, so the CPS node metadata is what tells a reader
//! how to interpret them. The metadata is built from the same
//! [`FormatDescriptor`] the batcher records locally, so the two cannot drift,
//! and the [`Anchor`](super::Anchor) only *executes* `set_meta` without
//! knowing what the values mean.

use crate::batch_store::FormatDescriptor;
use serde_json::{json, Value};

/// Builds the CPS metadata describing the payload format.
pub struct MetadataBuilder;

impl MetadataBuilder {
    /// Metadata for payloads encoded as `descriptor`, as UTF-8 JSON:
    ///
    /// ```json
    /// {"schema":"SignedEnvelopeBatch","schema_version":1,
    ///  "encoding":"protobuf","compression":"zstd"}
    /// ```
    pub fn build(descriptor: &FormatDescriptor) -> Vec<u8> {
        serde_json::to_vec(&Self::value(descriptor)).expect("JSON value serializes")
    }

    /// The metadata as a JSON value.
    pub fn value(descriptor: &FormatDescriptor) -> Value {
        json!({
            "schema": descriptor.schema,
            "schema_version": descriptor.schema_version,
            "encoding": descriptor.encoding,
            "compression": descriptor.compression,
        })
    }

    /// Whether `current` on-chain metadata already equals the desired
    /// metadata. Compared as JSON so key order and whitespace do not matter;
    /// metadata that is not valid JSON never matches.
    pub fn matches(descriptor: &FormatDescriptor, current: &[u8]) -> bool {
        serde_json::from_slice::<Value>(current)
            .map(|value| value == Self::value(descriptor))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(compression: &str) -> FormatDescriptor {
        FormatDescriptor {
            schema: "SignedEnvelopeBatch".into(),
            schema_version: 1,
            encoding: "protobuf".into(),
            compression: compression.into(),
        }
    }

    #[test]
    fn describes_the_payload_format() {
        let meta: Value =
            serde_json::from_slice(&MetadataBuilder::build(&descriptor("zstd"))).unwrap();
        assert_eq!(
            meta,
            json!({
                "schema": "SignedEnvelopeBatch",
                "schema_version": 1,
                "encoding": "protobuf",
                "compression": "zstd",
            })
        );
    }

    #[test]
    fn compression_none_is_stated_explicitly() {
        let meta = MetadataBuilder::value(&descriptor("none"));
        assert_eq!(meta["compression"], "none");
    }

    #[test]
    fn fits_the_cps_metadata_limit() {
        assert!(MetadataBuilder::build(&descriptor("zstd")).len() <= libcps::MAX_META_SIZE);
    }

    #[test]
    fn matches_ignores_formatting_but_not_content() {
        let d = descriptor("zstd");
        let reordered = br#"{ "compression": "zstd", "encoding": "protobuf",
            "schema_version": 1, "schema": "SignedEnvelopeBatch" }"#;
        assert!(MetadataBuilder::matches(&d, reordered));
        assert!(!MetadataBuilder::matches(&descriptor("none"), reordered));
        assert!(!MetadataBuilder::matches(&d, b"not json"));
    }
}
