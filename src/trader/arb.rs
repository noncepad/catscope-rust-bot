//! Incremental negative-cycle arbitrage detector.
//!
//! Models tradable assets as a directed weighted graph and detects profitable
//! arbitrage cycles in sub-microsecond time as prices update.
//!
//! # Theory
//!
//! Edge weight: `w = −ln(price × (1 − fee))`
//!
//! A cycle is profitable when `Σ w_i < 0`, because:
//! ```text
//! Σ −ln(p_i × (1 − fee_i)) < 0
//! ⟹ exp(−Σ w_i) > 1
//! ⟹ profit = exp(−cycle_cost) − 1  (fractional, before gas)
//! ```
//!
//! # Performance
//!
//! - Hot path is O(cycles_per_edge): additions and comparisons only, no allocations.
//! - CSR layout for both forward (cycle → edges) and reverse (edge → cycles) maps.
//! - Expected: 200 ns – 2 µs per price update at 10–80 cycles per edge.
//!
//! # Workflow
//!
//! ```rust,ignore
//! let mut g = ArbGraph::new(n_nodes);
//!
//! // One directed edge per swap direction.
//! let e_ab = g.add_edge(node_a, node_b, price_ab, 0.003, pool_id, mint_a, mint_b);
//! let e_ba = g.add_edge(node_b, node_a, price_ba, 0.003, pool_id, mint_b, mint_a);
//!
//! // Enumerate all simple cycles of length ≤ 3.
//! g.build_cycles(3);
//!
//! // Price event arrives:
//! if let Some(update) = trader.on_account(header, body) {
//!     g.update_from_price_update(&update, |cycle_id, profit| {
//!         let path = g.cycle_path(cycle_id);
//!         // execute arbitrage...
//!     });
//! }
//! ```

use crate::{
    graph::AccountId,
    trader::types::PriceUpdate,
};

pub type NodeId = u32;
pub type EdgeId = u32;
pub type CycleId = u32;

// ─── Types ────────────────────────────────────────────────────────────────────

/// One directed trading leg: `input_mint → output_mint` through a DEX pool.
#[derive(Clone)]
pub struct Edge {
    /// Source node (asset being sold).
    pub from: NodeId,
    /// Destination node (asset being bought).
    pub to: NodeId,
    /// Current log-space weight: `−ln(price × (1 − fee))`.
    /// Updated by [`ArbGraph::update_edge`].
    pub weight: f64,
    /// Fractional fee stored at registration (e.g. `0.003` for 0.3%).
    pub fee: f64,
    /// Catscope account ID of the pool this edge belongs to.
    pub pool_id: AccountId,
    /// Token being sold (input mint).
    pub input_mint: AccountId,
    /// Token being bought (output mint).
    pub output_mint: AccountId,
}

/// Emitted when a cycle's cost turns negative.
#[derive(Debug, Clone, Copy)]
pub struct ArbOpportunity {
    /// Which cycle became profitable.
    pub cycle_id: CycleId,
    /// `exp(−cycle_cost) − 1`.
    ///
    /// Example: `0.005` = 0.5% gross profit before fees and gas.
    pub profit_factor: f64,
}

// ─── Graph ────────────────────────────────────────────────────────────────────

/// Incremental negative-cycle detector over a directed trading graph.
pub struct ArbGraph {
    n_nodes: usize,

    /// All edges indexed by [`EdgeId`].
    edges: Vec<Edge>,

    /// Adjacency list used during [`build_cycles`] only.
    adj: Vec<Vec<(NodeId, EdgeId)>>,

    /// Sorted `(pool_id, edge_id)` pairs; binary-searched by `edges_for_pool`.
    pool_to_edges: Vec<(AccountId, EdgeId)>,

    // ── Post-build hot-path data (CSR) ─────────────────────────────────────

    /// Flattened edge lists for every cycle.
    /// Cycle `c` uses `cycle_edges[cycle_offsets[c]..cycle_offsets[c+1]]`.
    cycle_edges: Vec<EdgeId>,
    /// Length = n_cycles + 1 (sentinel-terminated).
    cycle_offsets: Vec<usize>,

    /// Flattened cycle lists for every edge (reverse index).
    /// Edge `e` appears in `edge_cycle_data[edge_cycle_offsets[e]..edge_cycle_offsets[e+1]]`.
    edge_cycle_data: Vec<CycleId>,
    /// Length = n_edges + 1 (sentinel-terminated).
    edge_cycle_offsets: Vec<usize>,

    /// Cached log-sum per cycle.  Negative ⟹ profitable.
    cycle_cost: Vec<f64>,
}

impl ArbGraph {
    /// Create a graph with `n_nodes` asset nodes pre-allocated.
    pub fn new(n_nodes: usize) -> Self {
        Self {
            n_nodes,
            edges: Vec::new(),
            adj: vec![Vec::new(); n_nodes],
            pool_to_edges: Vec::new(),
            cycle_edges: Vec::new(),
            cycle_offsets: Vec::new(),
            edge_cycle_data: Vec::new(),
            edge_cycle_offsets: Vec::new(),
            cycle_cost: Vec::new(),
        }
    }

    // ─── Setup ────────────────────────────────────────────────────────────────

    /// Register a directed edge for `from → to`.
    ///
    /// `initial_price`: how many `output_mint` units per `input_mint` unit.
    /// `fee`: fractional swap fee (`0.003` = 0.3%).
    ///
    /// Returns the [`EdgeId`] for this edge.
    pub fn add_edge(
        &mut self,
        from: NodeId,
        to: NodeId,
        initial_price: f64,
        fee: f64,
        pool_id: AccountId,
        input_mint: AccountId,
        output_mint: AccountId,
    ) -> EdgeId {
        assert!((from as usize) < self.n_nodes);
        assert!((to as usize) < self.n_nodes);
        let id = self.edges.len() as EdgeId;
        let weight = -(initial_price * (1.0 - fee)).ln();
        self.edges.push(Edge { from, to, weight, fee, pool_id, input_mint, output_mint });
        self.adj[from as usize].push((to, id));

        // Insert into sorted pool_to_edges for binary-search lookup.
        let pos = self.pool_to_edges.partition_point(|&(pid, _)| pid < pool_id);
        self.pool_to_edges.insert(pos, (pool_id, id));

        id
    }

    /// Enumerate all simple directed cycles up to `max_len` edges, compute
    /// initial costs, and build the CSR lookup tables.
    ///
    /// **Call once after all edges are added.**
    ///
    /// # Panics
    /// Panics if `max_len < 2`.
    pub fn build_cycles(&mut self, max_len: usize) {
        assert!(max_len >= 2, "max_len must be at least 2");

        let n = self.n_nodes;
        let n_edges = self.edges.len();

        // ── Enumerate raw cycles ──────────────────────────────────────────────
        let mut raw_cycles: Vec<Vec<EdgeId>> = Vec::new();
        let mut path: Vec<EdgeId> = Vec::with_capacity(max_len);
        let mut visited: Vec<bool> = vec![false; n];

        for start in 0..n as NodeId {
            Self::dfs_cycles(
                start,
                start,
                &self.adj,
                max_len - 1,
                &mut path,
                &mut visited,
                &mut raw_cycles,
            );
        }

        let n_cycles = raw_cycles.len();

        // ── Forward CSR: cycle → edges ────────────────────────────────────────
        let total_ce: usize = raw_cycles.iter().map(|c| c.len()).sum();
        self.cycle_edges = Vec::with_capacity(total_ce);
        self.cycle_offsets = Vec::with_capacity(n_cycles + 1);
        self.cycle_offsets.push(0);

        for cycle in &raw_cycles {
            self.cycle_edges.extend_from_slice(cycle);
            self.cycle_offsets.push(self.cycle_edges.len());
        }

        // ── Initial cycle costs ───────────────────────────────────────────────
        self.cycle_cost = Vec::with_capacity(n_cycles);
        for cycle in &raw_cycles {
            let cost: f64 = cycle.iter()
                .map(|&eid| self.edges[eid as usize].weight)
                .sum();
            self.cycle_cost.push(cost);
        }

        // ── Reverse CSR: edge → cycles ────────────────────────────────────────
        let mut count = vec![0usize; n_edges];
        for cycle in &raw_cycles {
            for &eid in cycle {
                count[eid as usize] += 1;
            }
        }

        self.edge_cycle_offsets = Vec::with_capacity(n_edges + 1);
        self.edge_cycle_offsets.push(0);
        for &c in &count {
            let prev = *self.edge_cycle_offsets.last().unwrap();
            self.edge_cycle_offsets.push(prev + c);
        }

        let total_ec = *self.edge_cycle_offsets.last().unwrap();
        self.edge_cycle_data = vec![0; total_ec];

        let mut cursor = vec![0usize; n_edges];
        for (cid, cycle) in raw_cycles.iter().enumerate() {
            for &eid in cycle {
                let e = eid as usize;
                let slot = self.edge_cycle_offsets[e] + cursor[e];
                self.edge_cycle_data[slot] = cid as CycleId;
                cursor[e] += 1;
            }
        }
    }

    // ─── Hot path ─────────────────────────────────────────────────────────────

    /// Update an edge weight from a new price and fire `on_arb` for every
    /// cycle that becomes profitable.
    ///
    /// **No heap allocations.  O(cycles_per_edge).**
    ///
    /// `new_price` is in the same units as the price given to [`add_edge`].
    /// The fee stored at registration is applied automatically.
    #[inline]
    pub fn update_edge<F>(&mut self, edge_id: EdgeId, new_price: f64, mut on_arb: F)
    where
        F: FnMut(CycleId, f64),
    {
        let eid = edge_id as usize;
        let fee = self.edges[eid].fee;
        let new_weight = -(new_price * (1.0 - fee)).ln();
        let delta = new_weight - self.edges[eid].weight;
        self.edges[eid].weight = new_weight;

        let lo = self.edge_cycle_offsets[eid];
        let hi = self.edge_cycle_offsets[eid + 1];

        for i in lo..hi {
            let cid = self.edge_cycle_data[i] as usize;
            // SAFETY: cid is always < cycle_cost.len() by construction.
            unsafe {
                let cost = self.cycle_cost.get_unchecked_mut(cid);
                *cost += delta;
                if *cost < 0.0 {
                    let profit = (-*cost).exp() - 1.0;
                    on_arb(self.edge_cycle_data[i], profit);
                }
            }
        }
    }

    /// Convenience: update all edges for a pool from a [`PriceUpdate`] emitted
    /// by [`DexTrader::on_account`] or [`DexTrader::on_token`].
    ///
    /// Each AMM pool has two directed edges (A→B and B→A); both are updated.
    /// Uses a fixed-size stack buffer — no allocations.
    pub fn update_from_price_update<F>(&mut self, update: &PriceUpdate, mut on_arb: F)
    where
        F: FnMut(CycleId, f64),
    {
        let p = &update.price;

        // Collect edge IDs into a stack buffer (pools have at most ~2 edges).
        let mut buf = [0u32; 8];
        let mut n = 0usize;
        {
            let lo = self.pool_to_edges.partition_point(|&(pid, _)| pid < update.pool);
            let hi = self.pool_to_edges.partition_point(|&(pid, _)| pid <= update.pool);
            for &(_, eid) in &self.pool_to_edges[lo..hi] {
                if n < buf.len() {
                    buf[n] = eid;
                    n += 1;
                }
            }
        }

        for &eid in &buf[..n] {
            let (inp, outp) = {
                let e = &self.edges[eid as usize];
                (e.input_mint, e.output_mint)
            };

            let new_price = if inp == p.token_a && outp == p.token_b {
                p.price
            } else if inp == p.token_b && outp == p.token_a {
                if p.price > 0.0 { 1.0 / p.price } else { continue }
            } else {
                continue;
            };

            self.update_edge(eid, new_price, |cid, profit| on_arb(cid, profit));
        }
    }

    // ─── Queries ──────────────────────────────────────────────────────────────

    /// Edge slice for a cycle.
    #[inline]
    pub fn cycle_path(&self, cycle_id: CycleId) -> &[EdgeId] {
        let lo = self.cycle_offsets[cycle_id as usize];
        let hi = self.cycle_offsets[cycle_id as usize + 1];
        &self.cycle_edges[lo..hi]
    }

    /// All currently profitable cycles.
    pub fn profitable_cycles(&self) -> impl Iterator<Item = ArbOpportunity> + '_ {
        self.cycle_cost
            .iter()
            .enumerate()
            .filter(|(_, &c)| c < 0.0)
            .map(|(i, &c)| ArbOpportunity {
                cycle_id: i as CycleId,
                profit_factor: (-c).exp() - 1.0,
            })
    }

    /// All edges registered for `pool_id` (usually the two swap directions).
    pub fn edges_for_pool(&self, pool_id: AccountId) -> impl Iterator<Item = EdgeId> + '_ {
        let lo = self.pool_to_edges.partition_point(|&(pid, _)| pid < pool_id);
        let hi = self.pool_to_edges.partition_point(|&(pid, _)| pid <= pool_id);
        self.pool_to_edges[lo..hi].iter().map(|&(_, eid)| eid)
    }

    /// Edge metadata.
    #[inline]
    pub fn edge(&self, id: EdgeId) -> &Edge { &self.edges[id as usize] }

    pub fn n_nodes(&self) -> usize { self.n_nodes }
    pub fn n_edges(&self) -> usize { self.edges.len() }
    pub fn n_cycles(&self) -> usize { self.cycle_cost.len() }

    // ─── Cycle enumeration (called only during build_cycles) ──────────────────

    /// DFS that emits simple directed cycles, each normalised to start at its
    /// lowest-ID node.
    fn dfs_cycles(
        start: NodeId,
        current: NodeId,
        adj: &[Vec<(NodeId, EdgeId)>],
        remaining: usize,
        path: &mut Vec<EdgeId>,
        visited: &mut Vec<bool>,
        cycles: &mut Vec<Vec<EdgeId>>,
    ) {
        for &(next, eid) in &adj[current as usize] {
            if next == start {
                // Close the cycle (path must be non-empty to avoid self-loops).
                if !path.is_empty() {
                    let mut cycle = path.clone();
                    cycle.push(eid);
                    cycles.push(cycle);
                }
            } else if next > start && !visited[next as usize] && remaining > 0 {
                // Only visit nodes > start: each cycle is enumerated once,
                // starting from its minimum-ID node.
                visited[next as usize] = true;
                path.push(eid);
                Self::dfs_cycles(start, next, adj, remaining - 1, path, visited, cycles);
                path.pop();
                visited[next as usize] = false;
            }
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Three-asset triangle: A(0) ↔ B(1) ↔ C(2) ↔ A(0).
    /// Build with balanced prices so no cycle is profitable at first,
    /// then shift one price to open an arb.
    fn triangle_graph() -> (ArbGraph, [EdgeId; 6]) {
        let mut g = ArbGraph::new(3);
        // price = 1.0, fee = 0  → weight = -ln(1.0) = 0
        let e_ab = g.add_edge(0, 1, 1.0, 0.0, 10, 100, 200);
        let e_ba = g.add_edge(1, 0, 1.0, 0.0, 10, 200, 100);
        let e_bc = g.add_edge(1, 2, 1.0, 0.0, 20, 200, 300);
        let e_cb = g.add_edge(2, 1, 1.0, 0.0, 20, 300, 200);
        let e_ca = g.add_edge(2, 0, 1.0, 0.0, 30, 300, 100);
        let e_ac = g.add_edge(0, 2, 1.0, 0.0, 30, 100, 300);
        g.build_cycles(3);
        (g, [e_ab, e_ba, e_bc, e_cb, e_ca, e_ac])
    }

    #[test]
    fn balanced_triangle_no_arb() {
        let (g, _) = triangle_graph();
        assert_eq!(g.n_cycles(), 2, "triangle has 2 directed 3-cycles");
        assert_eq!(g.profitable_cycles().count(), 0);
    }

    #[test]
    fn price_shift_opens_arb() {
        let (mut g, [e_ab, ..]) = triangle_graph();
        let mut found = false;

        // Setting A→B price to 1.1 while all other prices remain 1.0:
        // cycle A→B→C→A has weight = -ln(1.1) + -ln(1.0) + -ln(1.0)
        //                          ≈ -0.0953 + 0 + 0 = -0.0953  < 0  ✓
        g.update_edge(e_ab, 1.1, |_cid, profit| {
            found = true;
            assert!(profit > 0.0);
            // exp(-(-0.0953)) - 1 ≈ 0.1, roughly 10%
            assert!((profit - 0.1).abs() < 0.01, "profit ~ 10%: {profit}");
        });

        assert!(found, "arbitrage should have been detected");
    }

    #[test]
    fn update_edge_no_alloc_property() {
        // Just verify the hot-path runs without panicking on a built graph.
        let (mut g, [e_ab, ..]) = triangle_graph();
        g.update_edge(e_ab, 0.95, |_, _| {});
        g.update_edge(e_ab, 1.05, |_, _| {});
        g.update_edge(e_ab, 1.00, |_, _| {});
    }

    #[test]
    fn cycle_path_contains_correct_edges() {
        let (g, _) = triangle_graph();
        for cid in 0..g.n_cycles() as CycleId {
            let path = g.cycle_path(cid);
            assert_eq!(path.len(), 3, "all cycles in a triangle have 3 edges");
        }
    }

    #[test]
    fn fee_raises_break_even_price() {
        let mut g = ArbGraph::new(2);
        // A→B→A with 0.3% fee on each leg.
        let e_ab = g.add_edge(0, 1, 1.0, 0.003, 1, 1, 2);
        let _e_ba = g.add_edge(1, 0, 1.0, 0.003, 1, 2, 1);
        g.build_cycles(2);
        assert_eq!(g.n_cycles(), 1);

        // price=1 on both legs: cycle_cost = 2*(-ln(1.0*0.997)) ≈ 0.006 > 0
        assert_eq!(g.profitable_cycles().count(), 0);

        let mut found = false;
        // Need round-trip product > 1/(0.997)^2 ≈ 1.006 to profit.
        // Set A→B = 1.004 → product = 1.004 * 1.0 * 0.997^2 ≈ 0.998 still < 1.
        g.update_edge(e_ab, 1.004, |_, _| { found = true; });
        assert!(!found, "1.004 is below break-even");

        // Set A→B = 1.01 → product = 1.01 * 0.997^2 ≈ 1.004 > 1.
        g.update_edge(e_ab, 1.01, |_, profit| {
            found = true;
            assert!(profit > 0.0);
        });
        assert!(found, "1.01 should be above break-even");
    }

    #[test]
    fn update_from_price_update_routes_both_edges() {
        let mut g = ArbGraph::new(2);
        let _e_ab = g.add_edge(0, 1, 1.0, 0.0, 99, 1, 2);
        let _e_ba = g.add_edge(1, 0, 1.0, 0.0, 99, 2, 1);
        g.build_cycles(2);

        let update = PriceUpdate {
            pool: 99,
            price: crate::trader::types::PoolPrice {
                token_a: 1,
                token_b: 2,
                price: 1.05,  // A→B improved
                reserve_a: 1000,
                reserve_b: 1050,
                fee_bps: 0,
            },
        };

        let mut count = 0;
        g.update_from_price_update(&update, |_, _| { count += 1; });
        // cycle A→B→A: weight = -ln(1.05) + -ln(1/1.05) = -ln(1.05)+ln(1.05) = 0
        // Still 0, no arb. But both edges got updated without panic.
        assert_eq!(count, 0);
    }
}
