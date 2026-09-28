//! Time-Expanded Directed Acyclic Graph -- implements `src/trader/TIME.md`'s
//! design directly: exploiting non-atomic yield disparities over a fixed
//! holding period (e.g. holding an LST vs. borrowing to mint one) by
//! splitting each token into time-bound `(Token, TimeStep)` nodes and
//! finding the cheapest (or most profitable) path between two of them.
//!
//! Because every edge either stays at the same time step (a spot swap) or
//! strictly increases it (yield accrual, borrow financing, unstake delay),
//! the graph can never contain a cycle -- time only moves forward. That
//! means the shortest-path problem this module solves is a genuine DAG
//! single-source-shortest-path (topological sort + one relaxation pass,
//! `O(V + E)`), not the cycle-tolerant Bellman-Ford
//! [`crate::trader::pricegraph::TradeRouter`]/[`crate::trader::spfa`] use
//! elsewhere in this codebase for ordinary (single-timestamp) spot
//! arbitrage -- see `TIME.md`'s own "Calculating Arbitrage in Approach B"
//! section for why Bellman-Ford doesn't apply here.
//!
//! Pure/no host-import dependency, same testability discipline as
//! `pricegraph.rs`/`spfa.rs`/`credit.rs` -- callers (e.g.
//! `leveragedloopv1`) are responsible for pulling real rates
//! (`Wallet`/`DexState`/`lst_staking_apy`, all of which need the live WASM
//! runtime) and building [`Edge`]s from them before calling into this
//! module.

use std::collections::{HashMap, VecDeque};

/// A token at a specific point in time, e.g. `(SOL, 0)` or `(jitoSOL, 1)`.
/// `token` is a plain symbol string, not a mint/`AccountId` -- this graph
/// operates on the small, curated candidate set a caller builds (e.g.
/// `leveragedloopv1::LST_CANDIDATES` plus SOL/USDC), not arbitrary
/// on-chain mints. `time_step` is a step index, not a real duration --
/// `0` = now, `1` = one holding period later, etc.; how much real time
/// one step represents is entirely up to whatever `Δt` the caller used
/// when computing each edge's weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Node {
    pub token: &'static str,
    pub time_step: u32,
}

impl Node {
    pub fn new(token: &'static str, time_step: u32) -> Self {
        Self { token, time_step }
    }
}

/// One directed edge, weighted per `TIME.md`'s edge-weight table:
///
/// | Edge type | Connection | Weight |
/// |---|---|---|
/// | Spot swap | `(A,t) -> (B,t)` | `-ln(R_spot(A->B))` |
/// | LST accrual | `(LST,t) -> (LST,t+Δt)` | `-ln(1 + y*Δt)` |
/// | Borrow financing | `(SOL,t) -> (SOL,t+Δt)` | `-ln(1 - r_borrow*Δt)` |
/// | Unstake delay | `(LST,t) -> (SOL,t+T)` | `-ln(1 + y*T) + fee_penalty` |
///
/// Building the right weight for each edge type is the caller's job (see
/// the `spot_edge_weight`/`yield_edge_weight`/`borrow_edge_weight`
/// helpers below) -- this struct just stores the result.
#[derive(Debug, Clone, Copy)]
pub struct Edge {
    pub from: Node,
    pub to: Node,
    pub weight: f64,
}

/// `-ln(spot_rate)` -- weight for a horizontal spot-swap edge. `spot_rate`
/// is how many units of the destination token one unit of the source
/// token buys (e.g. `TradeRouter::route_slippage_aware`'s real quote,
/// `amount_out / amount_in` in a common USD-normalized unit).
pub fn spot_edge_weight(spot_rate: f64) -> f64 {
    -spot_rate.ln()
}

/// `-ln(1 + yield_apy * delta_t_years)` -- weight for a vertical LST
/// yield-accrual edge. `yield_apy` is a fraction (e.g. `0.073` for
/// 7.3%/yr, matching `CustomMessageInbound::LstApy`'s real units).
pub fn yield_edge_weight(yield_apy: f64, delta_t_years: f64) -> f64 {
    -(1.0 + yield_apy * delta_t_years).ln()
}

/// `-ln(1 - borrow_apy * delta_t_years)` -- weight for a borrow-financing
/// edge. `borrow_apy` is a fraction (e.g. `0.0433` for 4.33%/yr, matching
/// `KaminoReserve::current_borrow_apy`'s real units).
pub fn borrow_edge_weight(borrow_apy: f64, delta_t_years: f64) -> f64 {
    -(1.0 - borrow_apy * delta_t_years).ln()
}

/// `exp(-total_weight)` -- converts a path's summed edge weight back into
/// a plain ratio: `> 1.0` means the path is profitable (you end up with
/// more of the target token than you'd get by doing nothing), `< 1.0`
/// means a real loss. Inverse of the `-ln(...)` in every edge-weight
/// helper above, per `TIME.md`'s own "Calculate Profit at Target" step.
pub fn profit_ratio(total_weight: f64) -> f64 {
    (-total_weight).exp()
}

/// The time-expanded graph itself -- just a flat edge list; nodes are
/// discovered from the edges that reference them.
#[derive(Debug, Clone, Default)]
pub struct TimeGraph {
    edges: Vec<Edge>,
}

impl TimeGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_edge(&mut self, from: Node, to: Node, weight: f64) {
        self.edges.push(Edge { from, to, weight });
    }

    /// Every distinct node referenced by at least one edge, in no
    /// particular order -- `topological_order` is what actually orders
    /// them.
    fn nodes(&self) -> Vec<Node> {
        let mut seen: Vec<Node> = Vec::with_capacity(self.edges.len() * 2);
        for e in &self.edges {
            if !seen.contains(&e.from) {
                seen.push(e.from);
            }
            if !seen.contains(&e.to) {
                seen.push(e.to);
            }
        }
        seen
    }

    /// Kahn's algorithm -- a real topological sort, not just "sort by
    /// time_step": `TIME.md`'s own algorithm description notes nodes at
    /// the *same* timestamp still need dependency ordering (e.g. a spot
    /// swap's input token before its output), which a naive sort-by-time
    /// can't guarantee for an arbitrary node-name pairing. Standard,
    /// well-understood, and correct for any DAG, not just this specific
    /// graph shape -- this module's whole premise (no Bellman-Ford
    /// needed) depends on the graph genuinely being acyclic, which every
    /// edge-weight helper above guarantees by construction (time strictly
    /// increases, or stays the same for a same-timestamp spot swap that
    /// itself never cycles back).
    fn topological_order(&self) -> Vec<Node> {
        let nodes = self.nodes();
        let mut in_degree: HashMap<Node, usize> = nodes.iter().map(|&n| (n, 0)).collect();
        for e in &self.edges {
            *in_degree.get_mut(&e.to).unwrap() += 1;
        }
        let mut queue: VecDeque<Node> = nodes.iter().copied().filter(|n| in_degree[n] == 0).collect();
        let mut order = Vec::with_capacity(nodes.len());
        while let Some(u) = queue.pop_front() {
            order.push(u);
            for e in self.edges.iter().filter(|e| e.from == u) {
                let d = in_degree.get_mut(&e.to).unwrap();
                *d -= 1;
                if *d == 0 {
                    queue.push_back(e.to);
                }
            }
        }
        order
    }

    /// DAG single-source shortest path from `source` to `target`, via
    /// topological order + one relaxation pass -- `TIME.md`'s
    /// "Algorithm: DAG Shortest Path via Topological Sort" directly.
    /// Returns the total path weight and the path itself (`source` first,
    /// `target` last), or `None` if `target` isn't reachable from
    /// `source` at all.
    pub fn shortest_path(&self, source: Node, target: Node) -> Option<(f64, Vec<Node>)> {
        let order = self.topological_order();
        let mut dist: HashMap<Node, f64> = HashMap::new();
        let mut parent: HashMap<Node, Node> = HashMap::new();
        dist.insert(source, 0.0);
        for u in order {
            let Some(&du) = dist.get(&u) else { continue };
            for e in self.edges.iter().filter(|e| e.from == u) {
                let nd = du + e.weight;
                if nd < *dist.get(&e.to).unwrap_or(&f64::INFINITY) {
                    dist.insert(e.to, nd);
                    parent.insert(e.to, u);
                }
            }
        }
        let &final_weight = dist.get(&target)?;
        let mut path = vec![target];
        let mut cur = target;
        while cur != source {
            let &p = parent.get(&cur)?;
            path.push(p);
            cur = p;
        }
        path.reverse();
        Some((final_weight, path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOL: &str = "SOL";
    const JITOSOL: &str = "jitoSOL";

    #[test]
    fn edge_weight_helpers_match_time_md_formulas() {
        // A 1.0 spot rate (no price change) contributes zero weight.
        assert!((spot_edge_weight(1.0) - 0.0).abs() < 1e-12);
        // 7.3%/yr yield held for exactly 1 year should discount the
        // weight by ln(1.073), i.e. profit_ratio of a lone yield edge
        // over 1 year is 1.073.
        let w = yield_edge_weight(0.073, 1.0);
        assert!((profit_ratio(w) - 1.073).abs() < 1e-9);
        // 4.33%/yr borrow cost for 1 year: profit_ratio should be
        // 1 - 0.0433 (paying back principal + interest costs you that
        // fraction).
        let w = borrow_edge_weight(0.0433, 1.0);
        assert!((profit_ratio(w) - (1.0 - 0.0433)).abs() < 1e-9);
    }

    /// Mirrors `TIME.md`'s own worked shape: SOL(t0) -> jitoSOL(t0) [spot
    /// swap] -> jitoSOL(t1) [yield accrual] -> SOL(t1) [spot swap back],
    /// vs. a direct SOL(t0) -> SOL(t1) unlevered "do nothing" path
    /// (weight 0, profit_ratio exactly 1.0) -- the LST loop should beat
    /// doing nothing whenever its net swap-fee-adjusted yield is
    /// positive, confirming the DAG actually finds the real, better path
    /// rather than just returning *some* path.
    #[test]
    fn lst_loop_path_beats_holding_sol_when_yield_is_real() {
        let mut g = TimeGraph::new();
        let sol_t0 = Node::new(SOL, 0);
        let jitosol_t0 = Node::new(JITOSOL, 0);
        let jitosol_t1 = Node::new(JITOSOL, 1);
        let sol_t1 = Node::new(SOL, 1);

        // Real-ish numbers: swap SOL->jitoSOL and back both lose a
        // small amount to fees/slippage (0.3% each way), jitoSOL yields
        // 7.3%/yr, held for exactly 1 year.
        g.add_edge(sol_t0, jitosol_t0, spot_edge_weight(0.997));
        g.add_edge(jitosol_t0, jitosol_t1, yield_edge_weight(0.073, 1.0));
        g.add_edge(jitosol_t1, sol_t1, spot_edge_weight(0.997));
        // The "do nothing" alternative: SOL(t0) -> SOL(t1) at zero cost.
        g.add_edge(sol_t0, sol_t1, 0.0);

        let (weight, path) = g.shortest_path(sol_t0, sol_t1).expect("path must exist");
        // The LST loop must win (lower weight = higher profit_ratio) --
        // 0.997 * 1.073 * 0.997 ≈ 1.0665, comfortably above the "do
        // nothing" alternative's flat 1.0.
        let ratio = profit_ratio(weight);
        assert!(ratio > 1.0, "expected a profitable path, got ratio={ratio}");
        assert!(
            (ratio - 0.997 * 1.073 * 0.997).abs() < 1e-6,
            "ratio={ratio} didn't match the expected compounded value"
        );
        // The winning path must be the 4-node LST loop, not the 2-node
        // "do nothing" shortcut.
        assert_eq!(path, vec![sol_t0, jitosol_t0, jitosol_t1, sol_t1]);
    }

    #[test]
    fn do_nothing_wins_when_yield_cannot_cover_swap_fees() {
        let mut g = TimeGraph::new();
        let sol_t0 = Node::new(SOL, 0);
        let jitosol_t0 = Node::new(JITOSOL, 0);
        let jitosol_t1 = Node::new(JITOSOL, 1);
        let sol_t1 = Node::new(SOL, 1);

        // Same shape, but a thin yield (0.1%/yr) that can't clear two
        // 0.3% swap fees -- "do nothing" should win instead.
        g.add_edge(sol_t0, jitosol_t0, spot_edge_weight(0.997));
        g.add_edge(jitosol_t0, jitosol_t1, yield_edge_weight(0.001, 1.0));
        g.add_edge(jitosol_t1, sol_t1, spot_edge_weight(0.997));
        g.add_edge(sol_t0, sol_t1, 0.0);

        let (weight, path) = g.shortest_path(sol_t0, sol_t1).expect("path must exist");
        assert!((profit_ratio(weight) - 1.0).abs() < 1e-9);
        assert_eq!(path, vec![sol_t0, sol_t1]);
    }

    #[test]
    fn unreachable_target_returns_none() {
        let mut g = TimeGraph::new();
        g.add_edge(Node::new(SOL, 0), Node::new(JITOSOL, 0), 0.0);
        assert!(g.shortest_path(Node::new(SOL, 0), Node::new(SOL, 1)).is_none());
    }

    #[test]
    fn topological_order_respects_time_steps() {
        let mut g = TimeGraph::new();
        let sol_t0 = Node::new(SOL, 0);
        let jitosol_t0 = Node::new(JITOSOL, 0);
        let jitosol_t1 = Node::new(JITOSOL, 1);
        g.add_edge(sol_t0, jitosol_t0, 0.0);
        g.add_edge(jitosol_t0, jitosol_t1, 0.0);
        let order = g.topological_order();
        let pos = |n: Node| order.iter().position(|&x| x == n).unwrap();
        assert!(pos(sol_t0) < pos(jitosol_t0));
        assert!(pos(jitosol_t0) < pos(jitosol_t1));
    }
}
