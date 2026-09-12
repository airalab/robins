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
//! The `whitelist` authorization policy.
//!
//! An in-memory set of authorized [`SensorId`]s loaded from a text file. Each
//! non-empty, non-comment line holds one sensor identity in either SS58 or hex
//! form (an optional `0x` prefix is accepted); `#` begins a comment.

use super::AuthPolicy;
use crate::config::AuthMode;
use crate::protocol::SensorId;
use std::collections::HashSet;
use std::io;
use std::path::Path;

/// Accept only sensors whose [`SensorId`] appears in the loaded allow-list.
#[derive(Clone, Debug, Default)]
pub struct Whitelist {
    /// The set of authorized sensor identities.
    allowed: HashSet<SensorId>,
}

impl Whitelist {
    /// Build a whitelist from an iterator of authorized identities.
    pub fn new(ids: impl IntoIterator<Item = SensorId>) -> Self {
        Self {
            allowed: ids.into_iter().collect(),
        }
    }

    /// Number of authorized identities.
    pub fn len(&self) -> usize {
        self.allowed.len()
    }

    /// Whether the allow-list is empty (authorizes nobody).
    pub fn is_empty(&self) -> bool {
        self.allowed.is_empty()
    }

    /// Load a whitelist from a file, one identity per line.
    ///
    /// Blank lines and `#` comments are ignored. Each remaining line is parsed
    /// as an SS58 address first, then as hex; an unparseable entry aborts the
    /// load with the offending line number.
    pub fn from_file(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        let contents = std::fs::read_to_string(path)?;
        Self::from_str_lines(&contents).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid whitelist {}: {err}", path.display()),
            )
        })
    }

    /// Parse a whitelist from raw file contents.
    fn from_str_lines(contents: &str) -> Result<Self, String> {
        let mut allowed = HashSet::new();
        for (index, raw) in contents.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let sensor_id =
                parse_sensor_id(line).map_err(|err| format!("line {}: {err}", index + 1))?;
            allowed.insert(sensor_id);
        }
        Ok(Self { allowed })
    }
}

/// Parse a single whitelist entry as SS58, falling back to hex.
fn parse_sensor_id(entry: &str) -> Result<SensorId, String> {
    if let Ok(id) = SensorId::from_ss58(entry) {
        return Ok(id);
    }
    let hex = entry.strip_prefix("0x").unwrap_or(entry);
    SensorId::from_hex(hex).map_err(|err| format!("not a valid ss58 or hex sensor_id ({err})"))
}

impl AuthPolicy for Whitelist {
    fn mode(&self) -> AuthMode {
        AuthMode::Whitelist
    }

    fn authorize(&self, sensor_id: &SensorId) -> bool {
        self.allowed.contains(sensor_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::SensorIdentity;

    fn sensor(seed: u8) -> SensorId {
        SensorIdentity::from_secret_bytes(&[seed; 32]).sensor_id()
    }

    #[test]
    fn authorizes_only_listed_sensors() {
        let allowed = sensor(1);
        let denied = sensor(2);
        let policy = Whitelist::new([allowed.clone()]);

        assert_eq!(policy.mode(), AuthMode::Whitelist);
        assert_eq!(policy.len(), 1);
        assert!(policy.authorize(&allowed));
        assert!(!policy.authorize(&denied));
    }

    #[test]
    fn parses_mixed_ss58_and_hex_with_comments() {
        let a = sensor(3);
        let b = sensor(4);
        let contents = format!(
            "# authorized sensors\n{}\n\n0x{}  # inline comment\n",
            a.to_ss58(),
            b.to_hex(),
        );
        let policy = Whitelist::from_str_lines(&contents).unwrap();
        assert_eq!(policy.len(), 2);
        assert!(policy.authorize(&a));
        assert!(policy.authorize(&b));
    }

    #[test]
    fn rejects_malformed_entry() {
        let err = Whitelist::from_str_lines("not-a-key").unwrap_err();
        assert!(err.contains("line 1"), "{err}");
    }

    #[test]
    fn empty_whitelist_denies_all() {
        let policy = Whitelist::default();
        assert!(policy.is_empty());
        assert!(!policy.authorize(&sensor(1)));
    }
}
