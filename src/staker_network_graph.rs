//! Staker network graph data (issue #456).
//!
//! Exposes structured graph data describing delegation, referral, and mirror
//! relationships between stakers, for external visualization tooling to
//! render network maps of how stakers are connected.
//!
//! # Storage
//!
//! `DataKey` sits at Soroban's 50-variant cap, so this uses raw
//! `Symbol`-keyed persistent storage, matching `balance.rs` and other feature
//! modules. Delegation and referral edges are read from the existing
//! `balance::get_delegate` / `balance::get_referees` state; mirror
//! relationships are a new, self-service mapping introduced here.

use soroban_sdk::{contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec};

use crate::admin;
use crate::balance;
use crate::vault::{VaultContract, VaultContractClient};

/// Maximum stakers returned per `staker_network_graph_data()` page.
pub const MAX_GRAPH_PAGE_SIZE: u32 = 50;

/// Persistent key prefix: `(MIRROR_TARGET_KEY, follower) -> leader`.
const MIRROR_TARGET_KEY: Symbol = symbol_short!("mirr_tgt");

/// Classification of a staker within the network graph.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum NodeType {
    RegularStaker,
    Delegator,
    Referrer,
    Leader,
    SubAdmin,
}

/// A single staker node in the network graph.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct GraphNode {
    pub address: Address,
    pub staked_amount: i128,
    pub node_type: NodeType,
}

/// The kind of relationship a `GraphEdge` represents.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum EdgeType {
    Delegation,
    Referral,
    Mirror,
}

/// A directed relationship between two stakers.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct GraphEdge {
    pub from: Address,
    pub to: Address,
    pub edge_type: EdgeType,
}

/// Full graph payload returned to visualization tooling.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct NetworkGraphData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

/// Registers (or updates) that `follower` is mirroring `leader`'s staking
/// strategy, so the relationship shows up as a `Mirror` edge in the graph.
pub fn set_mirror_target(env: &Env, follower: &Address, leader: &Address) {
    env.storage()
        .persistent()
        .set(&(MIRROR_TARGET_KEY, follower.clone()), leader);
}

pub fn get_mirror_target(env: &Env, follower: &Address) -> Option<Address> {
    env.storage()
        .persistent()
        .get(&(MIRROR_TARGET_KEY, follower.clone()))
}

/// The staker with the largest staked amount among `stakers` is labeled
/// `Leader`; the configured emergency admin (if any) is labeled `SubAdmin`;
/// otherwise a staker is `Referrer` if they have referees, `Delegator` if
/// they've delegated their stake, and `RegularStaker` by default.
fn classify_node(env: &Env, address: &Address, leader: Option<&Address>) -> NodeType {
    if let Some(emergency) = admin::get_emergency_admin(env) {
        if &emergency == address {
            return NodeType::SubAdmin;
        }
    }
    if let Some(leader_addr) = leader {
        if leader_addr == address {
            return NodeType::Leader;
        }
    }
    if !balance::get_referees(env, address).is_empty() {
        return NodeType::Referrer;
    }
    if balance::get_delegate(env, address).is_some() {
        return NodeType::Delegator;
    }
    NodeType::RegularStaker
}

pub fn build_graph_data(env: &Env, stakers: &Vec<Address>) -> NetworkGraphData {
    let mut nodes = Vec::new(env);
    let mut edges = Vec::new(env);

    let mut leader_addr: Option<Address> = None;
    let mut leader_amount: i128 = -1;
    for addr in stakers.iter() {
        let amount = balance::get_shares(env, &addr);
        if amount > leader_amount {
            leader_amount = amount;
            leader_addr = Some(addr.clone());
        }
    }

    for addr in stakers.iter() {
        let staked_amount = balance::get_shares(env, &addr);
        let node_type = classify_node(env, &addr, leader_addr.as_ref());
        nodes.push_back(GraphNode {
            address: addr.clone(),
            staked_amount,
            node_type,
        });

        if let Some(delegate) = balance::get_delegate(env, &addr) {
            edges.push_back(GraphEdge {
                from: addr.clone(),
                to: delegate,
                edge_type: EdgeType::Delegation,
            });
        }

        for referee in balance::get_referees(env, &addr).iter() {
            edges.push_back(GraphEdge {
                from: addr.clone(),
                to: referee,
                edge_type: EdgeType::Referral,
            });
        }

        if let Some(mirror_leader) = get_mirror_target(env, &addr) {
            edges.push_back(GraphEdge {
                from: addr.clone(),
                to: mirror_leader,
                edge_type: EdgeType::Mirror,
            });
        }
    }

    NetworkGraphData { nodes, edges }
}

#[cfg_attr(not(feature = "testutils"), contractimpl)]
impl VaultContract {
    /// Issue #456: Returns structured graph data (nodes + edges) describing
    /// delegation, referral, and mirror relationships between the given
    /// stakers, for visualization tooling. Capped at `MAX_GRAPH_PAGE_SIZE`
    /// addresses per call.
    pub fn staker_network_graph_data(env: Env, addresses: Vec<Address>) -> NetworkGraphData {
        if addresses.len() <= MAX_GRAPH_PAGE_SIZE {
            return crate::staker_network_graph::build_graph_data(&env, &addresses);
        }

        let mut capped = Vec::new(&env);
        for i in 0..MAX_GRAPH_PAGE_SIZE {
            if let Some(addr) = addresses.get(i) {
                capped.push_back(addr);
            }
        }
        crate::staker_network_graph::build_graph_data(&env, &capped)
    }

    /// Issue #456: Lets a staker declare they are mirroring another staker's
    /// strategy, so the relationship is included as a `Mirror` edge in
    /// `staker_network_graph_data()`.
    pub fn mirror_staker(env: Env, follower: Address, leader: Address) {
        follower.require_auth();
        crate::staker_network_graph::set_mirror_target(&env, &follower, &leader);
    }

    /// Issue #456: Read-only query for who (if anyone) `follower` is
    /// currently mirroring.
    pub fn get_mirror_target(env: Env, follower: Address) -> Option<Address> {
        crate::staker_network_graph::get_mirror_target(&env, &follower)
    }
}
