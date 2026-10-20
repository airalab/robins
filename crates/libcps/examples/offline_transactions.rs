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
//! Build and validate CPS transactions offline.
//!
//! Transactions are plain prepared values: building one never touches the
//! network and needs no signer. This example uses only the free function
//! [`libcps::create_root`] and the [`CreateNode`] builder, so it runs without a
//! chain; the node handles (`cps.node(id).set_payload(..)`, `grant`, `create_scope`,
//! ...) build their transactions the same way, see `scope_access` and
//! `address_identity`.
//!
//! It shows size validation at build time, identity assignment parameters and how
//! the transaction type carries its result type.
//!
//! ```text
//! cargo run -p libcps --example offline_transactions --no-default-features
//! ```

use libcps::{
    AccountId, CreateNode, Error, IdentityAssignment, IdentityPermit, NodeId, PermitSignature,
    Transaction, MAX_META_SIZE, MAX_PAYLOAD_SIZE,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // A node creation builds to `Transaction<NodeId>`: the id of the new node is the
    // result once the transaction is finalized (see `TxReceipt::result`).
    let tx: Transaction<NodeId> = libcps::create_root(
        CreateNode::new()
            .meta(br#"{"type":"building"}"#)
            .payload(br#"{"status":"online"}"#),
    )?;
    println!("built: {tx:?}");

    // The builder methods accept anything convertible to bytes.
    let params = CreateNode::new().meta("text meta").payload(vec![1, 2, 3]);
    println!("meta {:?}, payload {:?}", params.meta, params.payload);

    // Limits mirror the runtime and are checked when the transaction is built, so
    // nothing oversized is ever sent.
    println!("limits: meta {MAX_META_SIZE} bytes, payload {MAX_PAYLOAD_SIZE} bytes");
    assert!(libcps::create_root(CreateNode::new().meta(vec![0; MAX_META_SIZE])).is_ok());
    match libcps::create_root(CreateNode::new().payload(vec![0; MAX_PAYLOAD_SIZE + 1])) {
        Err(Error::PayloadTooLarge { actual, max }) => {
            println!("rejected: payload is {actual} bytes, the maximum is {max}")
        }
        other => panic!("unexpected: {:?}", other.map(|_| ())),
    }

    // Identity of a new node: the signer's own account needs no permit ...
    let device = AccountId::from([7u8; 32]);
    let own = IdentityAssignment::new(device);
    assert!(own.permit.is_none());

    // ... any other account must provide a permit it signed for the scope. The
    // runtime defines the signed payload; the signature is supplied by the caller.
    let permit = IdentityPermit {
        valid_until: 1_000,
        signature: PermitSignature::Sr25519([0u8; 64]),
    };
    let delegated = IdentityAssignment::with_permit(device, permit);
    let child_params = CreateNode::new().identity(delegated);
    println!("child creation with identity: {:?}", child_params.identity);
    Ok(())
}
