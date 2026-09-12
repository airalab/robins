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
//! Bounded, time-to-live deduplication cache.
//!
//! The dedup key is the [`EnvelopeId`] — the SHA-256 of the *exact* received
//! envelope bytes — so the same signed message deduplicates across every
//! ingress transport (HTTP today, Meshtastic later). The cache bounds both time
//! and space:
//!
//! - **TTL**: an entry older than the configured window is treated as new,
//!   giving a sliding replay-suppression window without unbounded growth.
//! - **Capacity**: at most `capacity` entries are retained; when full, the
//!   oldest entry is evicted (LRU-by-insertion).
//!
//! Because the TTL is uniform, insertion order equals expiry order, so a single
//! FIFO queue drives both expiry pruning and capacity eviction in amortised
//! `O(1)`.

use crate::protocol::EnvelopeId;
use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

/// A bounded, TTL-based set of recently seen [`EnvelopeId`]s.
#[derive(Debug)]
pub struct DedupCache {
    /// Maximum number of retained entries.
    capacity: usize,
    /// How long an entry suppresses duplicates.
    ttl: Duration,
    /// Membership set for `O(1)` duplicate checks.
    seen: HashSet<EnvelopeId>,
    /// Insertion-ordered queue of `(id, expiry)` for pruning and eviction.
    order: VecDeque<(EnvelopeId, Instant)>,
}

impl DedupCache {
    /// Create a cache holding at most `capacity` entries for `ttl` each.
    ///
    /// `capacity` is clamped to at least one so the cache always suppresses at
    /// least the immediately preceding message.
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        let capacity = capacity.max(1);
        Self {
            capacity,
            ttl,
            seen: HashSet::with_capacity(capacity),
            order: VecDeque::with_capacity(capacity),
        }
    }

    /// Number of live entries currently retained.
    pub fn len(&self) -> usize {
        self.seen.len()
    }

    /// Whether the cache currently holds no entries.
    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }

    /// Record `id`, returning `true` if it is new (should be processed) or
    /// `false` if it duplicates a still-live entry.
    pub fn insert_if_new(&mut self, id: EnvelopeId) -> bool {
        self.insert_if_new_at(id, Instant::now())
    }

    /// [`insert_if_new`](Self::insert_if_new) with an explicit clock, for tests.
    fn insert_if_new_at(&mut self, id: EnvelopeId, now: Instant) -> bool {
        self.prune_expired(now);

        if self.seen.contains(&id) {
            return false;
        }

        self.seen.insert(id);
        self.order.push_back((id, now + self.ttl));
        self.enforce_capacity();
        true
    }

    /// Drop entries whose TTL has elapsed. Front-to-back because uniform TTL
    /// keeps `order` sorted by expiry.
    fn prune_expired(&mut self, now: Instant) {
        while let Some(&(id, expiry)) = self.order.front() {
            if expiry > now {
                break;
            }
            self.order.pop_front();
            self.seen.remove(&id);
        }
    }

    /// Evict the oldest entries until within `capacity`.
    fn enforce_capacity(&mut self) {
        while self.seen.len() > self.capacity {
            if let Some((id, _)) = self.order.pop_front() {
                self.seen.remove(&id);
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> EnvelopeId {
        // `envelope_id` hashes bytes; a distinct input yields a distinct id.
        crate::protocol::envelope_id(&[byte])
    }

    #[test]
    fn first_sight_is_new_repeat_is_duplicate() {
        let mut cache = DedupCache::new(16, Duration::from_secs(60));
        let now = Instant::now();
        assert!(cache.insert_if_new_at(id(1), now));
        assert!(!cache.insert_if_new_at(id(1), now));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn distinct_ids_are_all_new() {
        let mut cache = DedupCache::new(16, Duration::from_secs(60));
        let now = Instant::now();
        assert!(cache.insert_if_new_at(id(1), now));
        assert!(cache.insert_if_new_at(id(2), now));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn entry_expires_after_ttl() {
        let ttl = Duration::from_secs(10);
        let mut cache = DedupCache::new(16, ttl);
        let t0 = Instant::now();
        assert!(cache.insert_if_new_at(id(1), t0));
        // Still within the window: duplicate.
        assert!(!cache.insert_if_new_at(id(1), t0 + Duration::from_secs(5)));
        // Past the window: treated as new again.
        assert!(cache.insert_if_new_at(id(1), t0 + Duration::from_secs(11)));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn capacity_evicts_oldest() {
        let mut cache = DedupCache::new(2, Duration::from_secs(60));
        let now = Instant::now();
        assert!(cache.insert_if_new_at(id(1), now));
        assert!(cache.insert_if_new_at(id(2), now));
        // Inserting a third entry evicts the oldest (id 1).
        assert!(cache.insert_if_new_at(id(3), now));
        assert_eq!(cache.len(), 2);
        // id 2 and id 3 are still retained.
        assert!(!cache.insert_if_new_at(id(2), now));
        assert!(!cache.insert_if_new_at(id(3), now));
        // id 1 was evicted, so it is seen as new again.
        assert!(cache.insert_if_new_at(id(1), now));
    }
}
