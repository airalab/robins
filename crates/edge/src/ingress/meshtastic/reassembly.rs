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
//! Bounded fragment reassembly for the Meshtastic Transport v1 wire format.
//!
//! Implements the reassembly rules from
//! `src/protobufs/transport/meshtastic/v1.md` §12, §15, §16 and §17: state is
//! keyed by `(mesh_sender, message_id)` (this process serves a single local
//! Meshtastic destination, so `ingress_scope` is implicit); fragments may
//! arrive out of order; byte-identical duplicates are ignored without
//! refreshing the idle timeout; conflicting fragments poison the assembly;
//! and the assembly is bounded by configurable global/per-sender pending
//! counts plus idle and absolute lifetimes.
//!
//! This module is deliberately synchronous and takes `now: Instant` as an
//! explicit parameter rather than reading the clock itself, so timeout
//! behavior is exactly reproducible in unit tests without real sleeps.
//!
//! Per-message size is bounded by `max_reassembled_bytes`, and the combined
//! `max_pending * max_reassembled_bytes` gives an implicit bound on total
//! buffered bytes across all senders, satisfying the spec's "bound buffered
//! bytes globally/per-sender" requirement (§17) without a separate byte-based
//! configuration knob.

use bytes::Bytes;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::frame::{FragmentFrame, MessageId};

/// Reassembler resource limits, sourced from `[meshtastic]` configuration.
#[derive(Clone, Copy, Debug)]
pub struct ReassemblerConfig {
    /// Maximum bytes retained for one reassembled payload.
    pub max_reassembled_bytes: usize,
    /// Maximum fragments accepted for one payload (`<= 16` per the wire
    /// format).
    pub max_fragments: usize,
    /// Maximum number of incomplete assemblies retained globally.
    pub max_pending: usize,
    /// Maximum number of incomplete assemblies retained per mesh sender.
    pub max_pending_per_sender: usize,
    /// Idle timeout: an assembly not advanced within this long is discarded.
    pub idle_timeout: Duration,
    /// Absolute timeout: an assembly older than this is discarded regardless
    /// of progress.
    pub absolute_timeout: Duration,
}

/// The result of feeding one fragment into the reassembler.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The fragment was accepted; the assembly is still incomplete.
    Progress,
    /// Every fragment was received and the message-id check passed. Carries
    /// the exact concatenated payload bytes.
    Complete(Bytes),
    /// A byte-identical fragment was already stored; ignored without
    /// affecting the idle timeout.
    DuplicateIgnored,
    /// A fragment for an existing index arrived with different bytes, or a
    /// later fragment declared a different `fragment_count`. The assembly
    /// was discarded.
    Conflict,
    /// The fragment was rejected outright; see [`RejectReason`].
    Rejected(RejectReason),
}

/// Why a fragment (or the completed assembly) was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectReason {
    /// `fragment_count` exceeded the configured/wire maximum.
    TooManyFragments,
    /// The global pending-assembly limit was reached.
    PendingLimitGlobal,
    /// The per-sender pending-assembly limit was reached.
    PendingLimitPerSender,
    /// The reassembled payload would exceed `max_reassembled_bytes`.
    Oversize,
    /// `SHA256(reassembled)[0:6] != message_id` after full reassembly.
    MessageIdMismatch,
}

/// Reassembly key: `(mesh_sender, message_id)`. `ingress_scope` is implicit
/// (one reassembler instance per local Meshtastic ingress connection).
type Key = (u32, MessageId);

/// In-progress reassembly state for one key.
struct Assembly {
    count: u8,
    fragments: Vec<Option<Bytes>>,
    received: usize,
    total_bytes: usize,
    created_at: Instant,
    last_progress: Instant,
}

/// Bounded, synchronous fragment reassembler.
pub struct Reassembler {
    config: ReassemblerConfig,
    assemblies: HashMap<Key, Assembly>,
    pending_per_sender: HashMap<u32, usize>,
}

impl Reassembler {
    /// Create an empty reassembler with the given resource limits.
    pub fn new(config: ReassemblerConfig) -> Self {
        Self {
            config,
            assemblies: HashMap::new(),
            pending_per_sender: HashMap::new(),
        }
    }

    /// Number of assemblies currently pending (for the `_pending` gauge).
    pub fn pending_len(&self) -> usize {
        self.assemblies.len()
    }

    /// Feed one already-parsed [`FragmentFrame`] from `mesh_sender` into the
    /// reassembler.
    pub fn accept(&mut self, now: Instant, mesh_sender: u32, frame: FragmentFrame) -> Outcome {
        let key = (mesh_sender, frame.message_id);

        if let Some(assembly) = self.assemblies.get_mut(&key) {
            if assembly.count != frame.count {
                self.discard(&key);
                return Outcome::Conflict;
            }

            let idx = frame.index as usize;
            match &assembly.fragments[idx] {
                Some(existing) if *existing == frame.body => {
                    // Identical duplicate: ignored, does not refresh timeout.
                    return Outcome::DuplicateIgnored;
                }
                Some(_) => {
                    self.discard(&key);
                    return Outcome::Conflict;
                }
                None => {}
            }

            if assembly.total_bytes + frame.body.len() > self.config.max_reassembled_bytes {
                self.discard(&key);
                return Outcome::Rejected(RejectReason::Oversize);
            }

            assembly.total_bytes += frame.body.len();
            assembly.fragments[idx] = Some(frame.body);
            assembly.received += 1;
            assembly.last_progress = now;

            if assembly.received == assembly.count as usize {
                return self.complete(&key);
            }
            return Outcome::Progress;
        }

        // No existing state: validate before committing new resources.
        if frame.count as usize > self.config.max_fragments {
            return Outcome::Rejected(RejectReason::TooManyFragments);
        }
        if self.assemblies.len() >= self.config.max_pending {
            return Outcome::Rejected(RejectReason::PendingLimitGlobal);
        }
        let sender_pending = self
            .pending_per_sender
            .get(&mesh_sender)
            .copied()
            .unwrap_or(0);
        if sender_pending >= self.config.max_pending_per_sender {
            return Outcome::Rejected(RejectReason::PendingLimitPerSender);
        }
        if frame.body.len() > self.config.max_reassembled_bytes {
            return Outcome::Rejected(RejectReason::Oversize);
        }

        let mut fragments = vec![None; frame.count as usize];
        let total_bytes = frame.body.len();
        let idx = frame.index as usize;
        fragments[idx] = Some(frame.body);

        self.assemblies.insert(
            key,
            Assembly {
                count: frame.count,
                fragments,
                received: 1,
                total_bytes,
                created_at: now,
                last_progress: now,
            },
        );
        *self.pending_per_sender.entry(mesh_sender).or_insert(0) += 1;

        if frame.count == 1 {
            // Unreachable via `frame::parse_frame` (which rejects
            // fragment_count == 1), but handled defensively.
            return self.complete(&key);
        }
        Outcome::Progress
    }

    /// Discard every assembly that has exceeded its idle or absolute
    /// timeout. Returns the number of assemblies evicted (for metrics).
    pub fn sweep_expired(&mut self, now: Instant) -> usize {
        let expired: Vec<Key> = self
            .assemblies
            .iter()
            .filter(|(_, assembly)| {
                now.saturating_duration_since(assembly.last_progress) >= self.config.idle_timeout
                    || now.saturating_duration_since(assembly.created_at)
                        >= self.config.absolute_timeout
            })
            .map(|(key, _)| *key)
            .collect();
        let count = expired.len();
        for key in expired {
            self.discard(&key);
        }
        count
    }

    /// Remove every assembly for `mesh_sender`, e.g. when the local
    /// Meshtastic session is lost and reassembly state must not survive a
    /// reconnect (spec-adjacent operational requirement, not wire framing).
    pub fn clear(&mut self) {
        self.assemblies.clear();
        self.pending_per_sender.clear();
    }

    /// Remove a completed/conflicted/expired assembly and release its
    /// per-sender accounting.
    fn discard(&mut self, key: &Key) {
        if self.assemblies.remove(key).is_some() {
            let sender = key.0;
            if let Some(count) = self.pending_per_sender.get_mut(&sender) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    self.pending_per_sender.remove(&sender);
                }
            }
        }
    }

    /// Concatenate a fully-received assembly, verify its message-id digest
    /// prefix, and remove the assembly state either way.
    ///
    /// Returns [`Outcome::Conflict`] instead of panicking if internal
    /// invariants are somehow violated (missing entry, or a fragment slot
    /// unexpectedly empty when `received == count`) rather than trusting
    /// that callers always uphold them.
    fn complete(&mut self, key: &Key) -> Outcome {
        let Some(assembly) = self.assemblies.get(key) else {
            return Outcome::Conflict;
        };
        let mut buf = Vec::with_capacity(assembly.total_bytes);
        for fragment in &assembly.fragments {
            match fragment {
                Some(bytes) => buf.extend_from_slice(bytes),
                None => {
                    self.discard(key);
                    return Outcome::Conflict;
                }
            }
        }
        self.discard(key);

        let digest = Sha256::digest(&buf);
        if digest[..super::frame::MESSAGE_ID_LEN] != key.1 {
            return Outcome::Rejected(RejectReason::MessageIdMismatch);
        }
        Outcome::Complete(Bytes::from(buf))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingress::meshtastic::frame::MAX_FRAGMENT_BODY;

    fn config() -> ReassemblerConfig {
        ReassemblerConfig {
            max_reassembled_bytes: 4096,
            max_fragments: 16,
            max_pending: 4,
            max_pending_per_sender: 2,
            idle_timeout: Duration::from_secs(60),
            absolute_timeout: Duration::from_secs(300),
        }
    }

    fn message_id(raw: &[u8]) -> MessageId {
        let digest = Sha256::digest(raw);
        let mut id = [0u8; 6];
        id.copy_from_slice(&digest[..6]);
        id
    }

    fn frag(message_id: MessageId, index: u8, count: u8, body: &[u8]) -> FragmentFrame {
        FragmentFrame {
            message_id,
            index,
            count,
            body: Bytes::from(body.to_vec()),
        }
    }

    #[test]
    fn single_fragment_message_completes() {
        // A "1 fragment worth of bytes" payload split across the minimum
        // fragment_count of 2 (fragment_count == 1 is rejected by the frame
        // parser, so the smallest fragmented case is 2 fragments).
        let raw = vec![0xaa; MAX_FRAGMENT_BODY + 1];
        let id = message_id(&raw);
        let mut r = Reassembler::new(config());
        let now = Instant::now();

        let out = r.accept(now, 1, frag(id, 0, 2, &raw[..MAX_FRAGMENT_BODY]));
        assert_eq!(out, Outcome::Progress);
        let out = r.accept(now, 1, frag(id, 1, 2, &raw[MAX_FRAGMENT_BODY..]));
        assert_eq!(out, Outcome::Complete(Bytes::from(raw)));
        assert_eq!(r.pending_len(), 0);
    }

    #[test]
    fn multi_fragment_out_of_order_reassembles_exactly() {
        let raw = (0u16..600).map(|i| (i % 256) as u8).collect::<Vec<u8>>();
        let id = message_id(&raw);
        let chunks: Vec<&[u8]> = raw.chunks(MAX_FRAGMENT_BODY).collect();
        let count = chunks.len() as u8;
        let mut r = Reassembler::new(config());
        let now = Instant::now();

        // Feed fragments in reverse order.
        let mut last = Outcome::Progress;
        for index in (0..chunks.len()).rev() {
            last = r.accept(now, 7, frag(id, index as u8, count, chunks[index]));
        }
        assert_eq!(last, Outcome::Complete(Bytes::from(raw)));
    }

    #[test]
    fn identical_duplicate_fragment_is_ignored() {
        let raw = vec![0x11u8; MAX_FRAGMENT_BODY + 5];
        let id = message_id(&raw);
        let mut r = Reassembler::new(config());
        let now = Instant::now();

        r.accept(now, 1, frag(id, 0, 2, &raw[..MAX_FRAGMENT_BODY]));
        let out = r.accept(now, 1, frag(id, 0, 2, &raw[..MAX_FRAGMENT_BODY]));
        assert_eq!(out, Outcome::DuplicateIgnored);

        let out = r.accept(now, 1, frag(id, 1, 2, &raw[MAX_FRAGMENT_BODY..]));
        assert_eq!(out, Outcome::Complete(Bytes::from(raw)));
    }

    #[test]
    fn conflicting_duplicate_fragment_poisons_assembly() {
        let raw = vec![0x22u8; MAX_FRAGMENT_BODY + 5];
        let id = message_id(&raw);
        let mut r = Reassembler::new(config());
        let now = Instant::now();

        r.accept(now, 1, frag(id, 0, 2, &raw[..MAX_FRAGMENT_BODY]));
        let mut different = vec![0x99u8; MAX_FRAGMENT_BODY];
        different[0] = 0x00;
        let out = r.accept(now, 1, frag(id, 0, 2, &different));
        assert_eq!(out, Outcome::Conflict);
        assert_eq!(r.pending_len(), 0);
    }

    #[test]
    fn fragment_count_mismatch_is_conflict() {
        let raw = vec![0x33u8; MAX_FRAGMENT_BODY + 5];
        let id = message_id(&raw);
        let mut r = Reassembler::new(config());
        let now = Instant::now();

        r.accept(now, 1, frag(id, 0, 2, &raw[..MAX_FRAGMENT_BODY]));
        let out = r.accept(now, 1, frag(id, 1, 3, &raw[MAX_FRAGMENT_BODY..]));
        assert_eq!(out, Outcome::Conflict);
        assert_eq!(r.pending_len(), 0);
    }

    #[test]
    fn same_message_id_from_different_senders_is_isolated() {
        let raw = vec![0x44u8; MAX_FRAGMENT_BODY + 5];
        let id = message_id(&raw);
        let mut r = Reassembler::new(config());
        let now = Instant::now();

        r.accept(now, 1, frag(id, 0, 2, &raw[..MAX_FRAGMENT_BODY]));
        // Sender 2's fragment 0 for the *same* message_id must not be seen
        // as a duplicate/conflict of sender 1's state.
        let out = r.accept(now, 2, frag(id, 0, 2, &raw[..MAX_FRAGMENT_BODY]));
        assert_eq!(out, Outcome::Progress);

        let out1 = r.accept(now, 1, frag(id, 1, 2, &raw[MAX_FRAGMENT_BODY..]));
        let out2 = r.accept(now, 2, frag(id, 1, 2, &raw[MAX_FRAGMENT_BODY..]));
        assert_eq!(out1, Outcome::Complete(Bytes::from(raw.clone())));
        assert_eq!(out2, Outcome::Complete(Bytes::from(raw)));
    }

    #[test]
    fn maximum_fragment_count_is_accepted() {
        let raw = vec![0x55u8; MAX_FRAGMENT_BODY * 16];
        let id = message_id(&raw);
        let mut r = Reassembler::new(config());
        let now = Instant::now();

        let mut last = Outcome::Progress;
        for index in 0..16u8 {
            let start = index as usize * MAX_FRAGMENT_BODY;
            last = r.accept(
                now,
                9,
                frag(id, index, 16, &raw[start..start + MAX_FRAGMENT_BODY]),
            );
        }
        assert_eq!(last, Outcome::Complete(Bytes::from(raw)));
    }

    #[test]
    fn fragment_count_over_configured_max_is_rejected() {
        let mut cfg = config();
        cfg.max_fragments = 4;
        let mut r = Reassembler::new(cfg);
        let now = Instant::now();
        let id = [0u8; 6];
        let out = r.accept(now, 1, frag(id, 0, 5, &[0u8; MAX_FRAGMENT_BODY]));
        assert_eq!(out, Outcome::Rejected(RejectReason::TooManyFragments));
        assert_eq!(r.pending_len(), 0);
    }

    #[test]
    fn envelope_exceeding_max_reassembled_bytes_is_rejected() {
        let mut cfg = config();
        cfg.max_reassembled_bytes = MAX_FRAGMENT_BODY + 10;
        let mut r = Reassembler::new(cfg);
        let now = Instant::now();
        let id = [1u8; 6];

        r.accept(now, 1, frag(id, 0, 2, &[0u8; MAX_FRAGMENT_BODY]));
        // Adding the second fragment's bytes would exceed the byte budget.
        let out = r.accept(now, 1, frag(id, 1, 2, &[0u8; 20]));
        assert_eq!(out, Outcome::Rejected(RejectReason::Oversize));
        assert_eq!(r.pending_len(), 0);
    }

    #[test]
    fn max_pending_global_is_enforced() {
        let mut cfg = config();
        cfg.max_pending = 1;
        cfg.max_pending_per_sender = 8;
        let mut r = Reassembler::new(cfg);
        let now = Instant::now();

        r.accept(now, 1, frag([1u8; 6], 0, 2, &[0u8; MAX_FRAGMENT_BODY]));
        let out = r.accept(now, 2, frag([2u8; 6], 0, 2, &[0u8; MAX_FRAGMENT_BODY]));
        assert_eq!(out, Outcome::Rejected(RejectReason::PendingLimitGlobal));
    }

    #[test]
    fn max_pending_per_sender_is_enforced() {
        let mut cfg = config();
        cfg.max_pending = 8;
        cfg.max_pending_per_sender = 1;
        let mut r = Reassembler::new(cfg);
        let now = Instant::now();

        r.accept(now, 1, frag([1u8; 6], 0, 2, &[0u8; MAX_FRAGMENT_BODY]));
        let out = r.accept(now, 1, frag([2u8; 6], 0, 2, &[0u8; MAX_FRAGMENT_BODY]));
        assert_eq!(out, Outcome::Rejected(RejectReason::PendingLimitPerSender));
    }

    #[test]
    fn idle_timeout_expires_incomplete_assembly() {
        let mut r = Reassembler::new(config());
        let now = Instant::now();
        r.accept(now, 1, frag([1u8; 6], 0, 2, &[0u8; MAX_FRAGMENT_BODY]));
        assert_eq!(r.pending_len(), 1);

        let later = now + Duration::from_secs(61);
        let evicted = r.sweep_expired(later);
        assert_eq!(evicted, 1);
        assert_eq!(r.pending_len(), 0);
    }

    #[test]
    fn absolute_timeout_expires_assembly_even_with_progress() {
        let mut r = Reassembler::new(config());
        let now = Instant::now();
        // 7 fragments so the assembly stays incomplete after 6 are
        // delivered, letting us keep "making progress" without ever
        // completing (which would remove the assembly before the absolute
        // timeout could apply).
        r.accept(now, 1, frag([1u8; 6], 0, 7, &[0u8; MAX_FRAGMENT_BODY]));

        // Deliver one new fragment every 59s (under the 60s idle timeout,
        // so idle-based eviction never triggers) until the absolute
        // timeout (300s from creation) is closing in.
        let mut t = now;
        for idx in 1..=5u8 {
            t += Duration::from_secs(59);
            r.accept(t, 1, frag([1u8; 6], idx, 7, &[0u8; MAX_FRAGMENT_BODY]));
            let evicted = r.sweep_expired(t);
            assert_eq!(evicted, 0, "must not expire before absolute_timeout");
        }
        // t is now 295s after creation; last progress was at t, well within
        // the idle timeout. Advancing another 10s (305s since creation)
        // must still expire the assembly on the absolute timeout alone.
        let t_over = t + Duration::from_secs(10);
        let evicted = r.sweep_expired(t_over);
        assert_eq!(
            evicted, 1,
            "absolute timeout must apply despite recent progress"
        );
        assert_eq!(r.pending_len(), 0);
    }

    #[test]
    fn message_id_mismatch_is_rejected() {
        let mut r = Reassembler::new(config());
        let now = Instant::now();
        let wrong_id = [0xffu8; 6]; // does not match SHA256 of the body below
        r.accept(now, 1, frag(wrong_id, 0, 2, &[0u8; MAX_FRAGMENT_BODY]));
        let out = r.accept(now, 1, frag(wrong_id, 1, 2, &[0u8; 10]));
        assert_eq!(out, Outcome::Rejected(RejectReason::MessageIdMismatch));
        assert_eq!(r.pending_len(), 0);
    }

    #[test]
    fn exact_byte_reconstruction() {
        let raw: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        let id = message_id(&raw);
        let mut r = Reassembler::new(config());
        let now = Instant::now();
        let chunks: Vec<&[u8]> = raw.chunks(MAX_FRAGMENT_BODY).collect();
        let count = chunks.len() as u8;

        let mut last = Outcome::Progress;
        for (index, chunk) in chunks.iter().enumerate() {
            last = r.accept(now, 3, frag(id, index as u8, count, chunk));
        }
        match last {
            Outcome::Complete(bytes) => assert_eq!(bytes.as_ref(), raw.as_slice()),
            other => panic!("expected completion, got {other:?}"),
        }
    }
}
