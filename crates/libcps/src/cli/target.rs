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
//! The common `<TARGET>` argument: a [`NodeId`] or an identity account.
//!
//! The syntax selects the projection of navigational reads (`parent`,
//! `children`, `tree`, `show`): a node target stays in `NodeId` space, an account
//! target stays in identity/account space and fails explicitly when a node of
//! the result has no identity. Mutations always [canonicalize](Target::node_id)
//! to a `NodeId` first.

use anyhow::Result;
use libcps::{AccountId, CpsAt, NodeId};
use serde_json::{json, Value};
use std::{fmt, future::Future, pin::Pin, str::FromStr};

/// A CPS node addressed by id or by identity account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Canonical node id.
    Node(NodeId),
    /// Identity account projection of a node.
    Address(AccountId),
}

impl FromStr for Target {
    type Err = String;

    /// Detection order: explicit `node:`/`addr:` prefix, then a `u64` node id,
    /// then an SS58 account.
    fn from_str(text: &str) -> Result<Self, String> {
        if let Some(rest) = text.strip_prefix("node:") {
            return rest
                .parse::<u64>()
                .map(|id| Self::Node(NodeId(id)))
                .map_err(|_| format!("invalid target `{text}`: expected a numeric node id"));
        }
        if let Some(rest) = text.strip_prefix("addr:") {
            return AccountId::from_str(rest)
                .map(Self::Address)
                .map_err(|_| format!("invalid target `{text}`: expected an SS58 account"));
        }
        if let Ok(id) = text.parse::<u64>() {
            return Ok(Self::Node(NodeId(id)));
        }
        AccountId::from_str(text)
            .map(Self::Address)
            .map_err(|_| format!("invalid target `{text}`: expected a node id or an SS58 account"))
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Node(id) => id.fmt(f),
            Self::Address(account) => account.fmt(f),
        }
    }
}

/// Parse an account argument (SS58 address or `0x` hex public key).
pub fn parse_account(text: &str) -> Result<AccountId, String> {
    libcps::identity::parse_account(text).map_err(|e| e.to_string())
}

impl Target {
    /// Value in the target's own projection: a number or an SS58 string.
    pub fn json(&self) -> Value {
        match self {
            Self::Node(id) => json!(id.0),
            Self::Address(account) => json!(account.to_string()),
        }
    }

    /// Canonical node of the target at the pinned block.
    ///
    /// # Errors
    ///
    /// `NodeNotFound` for a missing node, `IdentityNotFound` for an account
    /// that is the identity of no node.
    pub async fn node_id(&self, at: &CpsAt) -> Result<NodeId> {
        match self {
            Self::Node(id) => {
                if at.node(*id).info().await?.is_none() {
                    return Err(libcps::Error::NodeNotFound(*id).into());
                }
                Ok(*id)
            }
            Self::Address(account) => at
                .address(*account)
                .node_id()
                .await?
                .ok_or_else(|| libcps::Error::IdentityNotFound(*account).into()),
        }
    }

    /// Parent in the target's projection, `None` for a root.
    ///
    /// # Errors
    ///
    /// Fails when the target does not exist or, in account projection, when
    /// the parent has no identity.
    pub async fn parent(&self, at: &CpsAt) -> Result<Option<Target>> {
        self.node_id(at).await?;
        Ok(match self {
            Self::Node(id) => at.node(*id).parent().await?.map(Self::Node),
            Self::Address(account) => at.address(*account).parent().await?.map(Self::Address),
        })
    }

    /// Children in the target's projection.
    ///
    /// # Errors
    ///
    /// Fails when the target does not exist or, in account projection, when
    /// any child has no identity: children are never silently omitted.
    pub async fn children(&self, at: &CpsAt) -> Result<Vec<Target>> {
        self.node_id(at).await?;
        Ok(match self {
            Self::Node(id) => at
                .node(*id)
                .children()
                .await?
                .into_iter()
                .map(Self::Node)
                .collect(),
            Self::Address(account) => at
                .address(*account)
                .children()
                .await?
                .into_iter()
                .map(Self::Address)
                .collect(),
        })
    }
}

/// Recursive subtree in one projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tree {
    /// Root of the subtree.
    pub target: Target,
    /// Children, in chain order.
    pub children: Vec<Tree>,
}

impl Tree {
    /// Read the subtree of `target` at one pinned block, descending at most
    /// `depth` levels below it (`None`: unlimited).
    pub fn read<'a>(
        at: &'a CpsAt,
        target: Target,
        depth: Option<usize>,
    ) -> Pin<Box<dyn Future<Output = Result<Tree>> + 'a>> {
        Box::pin(async move {
            let mut children = Vec::new();
            if depth != Some(0) {
                for child in target.children(at).await? {
                    children.push(Self::read(at, child, depth.map(|d| d - 1)).await?);
                }
            } else {
                target.node_id(at).await?;
            }
            Ok(Tree { target, children })
        })
    }

    /// Nested JSON: `{"id": <node or account>, "children": [...]}`.
    pub fn json(&self) -> Value {
        json!({
            "id": self.target.json(),
            "children": self.children.iter().map(Tree::json).collect::<Vec<_>>(),
        })
    }

    /// Box-drawing rendering.
    pub fn render(&self) -> String {
        let mut out = self.target.to_string();
        Self::render_children(&self.children, "", &mut out);
        out
    }

    fn render_children(children: &[Tree], prefix: &str, out: &mut String) {
        for (i, child) in children.iter().enumerate() {
            let last = i + 1 == children.len();
            out.push_str(&format!(
                "\n{prefix}{} {}",
                if last { "└──" } else { "├──" },
                child.target
            ));
            let next = format!("{prefix}{}", if last { "    " } else { "│   " });
            Self::render_children(&child.children, &next, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALICE: &str = "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY";

    #[test]
    fn numbers_are_nodes_and_ss58_are_accounts() {
        assert_eq!("42".parse::<Target>().unwrap(), Target::Node(NodeId(42)));
        assert!(matches!(
            ALICE.parse::<Target>().unwrap(),
            Target::Address(_)
        ));
    }

    #[test]
    fn explicit_prefixes_override_detection() {
        assert_eq!("node:7".parse::<Target>().unwrap(), Target::Node(NodeId(7)));
        assert_eq!(
            format!("addr:{ALICE}").parse::<Target>().unwrap(),
            ALICE.parse::<Target>().unwrap()
        );
        assert!("node:5Grw".parse::<Target>().is_err());
        assert!("addr:7".parse::<Target>().is_err());
    }

    #[test]
    fn garbage_is_an_invalid_target() {
        let err = "not-a-target".parse::<Target>().unwrap_err();
        assert!(err.contains("invalid target"), "{err}");
        assert!("-1".parse::<Target>().is_err());
        assert!("".parse::<Target>().is_err());
    }

    #[test]
    fn display_keeps_the_projection() {
        assert_eq!("42".parse::<Target>().unwrap().to_string(), "42");
        assert_eq!(ALICE.parse::<Target>().unwrap().to_string(), ALICE);
    }

    #[test]
    fn tree_renders_with_branches() {
        let leaf = |n| Tree {
            target: Target::Node(NodeId(n)),
            children: vec![],
        };
        let tree = Tree {
            target: Target::Node(NodeId(1)),
            children: vec![
                Tree {
                    target: Target::Node(NodeId(2)),
                    children: vec![leaf(5), leaf(6)],
                },
                leaf(3),
            ],
        };
        assert_eq!(tree.render(), "1\n├── 2\n│   ├── 5\n│   └── 6\n└── 3");
        assert_eq!(tree.json()["children"][0]["children"][1]["id"], 6);
    }
}
