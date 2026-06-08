//! Fair-price graph based on Bellman-Ford over log exchange rates.
//!
//! Nodes are token mints (`AccountId`); directed edges are swap routes
//! weighted by `-ln(effective_rate)`, where
//! `effective_rate = spot_price × (1 − fee)`.
//!
//! Bellman-Ford finds the minimum-cost path, which corresponds to the
//! maximum cumulative exchange rate from a reference token to every other
//! reachable token.
//!
//! # Usage
//!
//! ```rust,ignore
//! let graph = PriceGraph::from_orca_pools(&state.m_orca_pool);
//! if let Some(prices) = graph.fair_prices(usdc_id) {
//!     for (token, price) in &prices {
//!         println!("token {token} = {price:.6} USDC");
//!     }
//! }
//! if graph.has_arbitrage() {
//!     // a profitable swap loop exists
//! }
//! ```

use crate::{graph::AccountId, trader::dex::orca::OrcaWhirlpool};
use std::collections::HashMap;

// ─── Internal edge ────────────────────────────────────────────────────────────

struct Edge {
    to: usize,
    /// `-ln(effective_rate)`: minimising this maximises the exchange rate.
    neg_log_rate: f64,
}

// ─── PriceGraph ───────────────────────────────────────────────────────────────

/// Token exchange-rate graph for fair-price and arbitrage calculation.
pub struct PriceGraph {
    token_index: HashMap<AccountId, usize>,
    tokens: Vec<AccountId>,
    /// Adjacency list indexed by token index.
    edges: Vec<Vec<Edge>>,
}

impl PriceGraph {
    /// Empty graph.
    pub fn new() -> Self {
        Self {
            token_index: HashMap::new(),
            tokens: Vec::new(),
            edges: Vec::new(),
        }
    }

    /// Build a price graph from a map of live Orca Whirlpool pool states.
    ///
    /// Pools with a zero spot price are skipped.
    pub fn from_orca_pools(pools: &HashMap<AccountId, OrcaWhirlpool>) -> Self {
        let mut g = Self::new();
        for pool in pools.values() {
            g.add_pool(
                pool.token_mint_a,
                pool.token_mint_b,
                pool.spot_price(),
                pool.fee_bps() as f64 / 10_000.0,
            );
        }
        g
    }

    /// Add a liquidity pool as two directed edges (A → B and B → A).
    ///
    /// `price_b_per_a`: spot price — units of B received per unit of A.
    /// `fee_rate`: fractional fee (e.g. `0.003` for 0.3 %).
    ///
    /// Silently ignores entries with non-positive prices or out-of-range fees.
    pub fn add_pool(
        &mut self,
        token_a: AccountId,
        token_b: AccountId,
        price_b_per_a: f64,
        fee_rate: f64,
    ) {
        if price_b_per_a <= 0.0 || !(0.0..1.0).contains(&fee_rate) {
            return;
        }
        let after_fee = 1.0 - fee_rate;
        let a = self.node(token_a);
        let b = self.node(token_b);
        self.edges[a].push(Edge {
            to: b,
            neg_log_rate: -(price_b_per_a * after_fee).ln(),
        });
        self.edges[b].push(Edge {
            to: a,
            neg_log_rate: -(after_fee / price_b_per_a).ln(),
        });
    }

    /// Compute fair prices of all tokens reachable from `reference` (= 1.0).
    ///
    /// Runs Bellman-Ford (n − 1 relaxations) from `reference`.
    /// Tokens not reachable from `reference` are absent from the result.
    ///
    /// Returns `None` only if `reference` is not present in the graph.
    /// To check for arbitrage (negative cycles), call [`has_arbitrage`].
    pub fn fair_prices(&self, reference: AccountId) -> Option<HashMap<AccountId, f64>> {
        let n = self.tokens.len();
        let &src = self.token_index.get(&reference)?;

        let mut dist = vec![f64::INFINITY; n];
        dist[src] = 0.0;

        for _ in 0..n.saturating_sub(1) {
            let mut changed = false;
            for u in 0..n {
                if dist[u].is_infinite() {
                    continue;
                }
                for edge in &self.edges[u] {
                    let d = dist[u] + edge.neg_log_rate;
                    if d < dist[edge.to] {
                        dist[edge.to] = d;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }

        let mut prices = HashMap::with_capacity(n);
        for (idx, &d) in dist.iter().enumerate() {
            if d.is_finite() {
                // price = e^(−dist): dist = 0 at reference → price = 1.0
                prices.insert(self.tokens[idx], (-d).exp());
            }
        }
        Some(prices)
    }

    /// Returns `true` if any negative cycle (arbitrage loop) exists.
    ///
    /// Uses a virtual-source variant so disconnected subgraphs are also checked:
    /// all nodes start at distance 0 and one extra relaxation pass is run after
    /// the standard n − 1 rounds. Any further improvement signals a negative cycle.
    pub fn has_arbitrage(&self) -> bool {
        let n = self.tokens.len();
        // All distances initialised to 0 acts like a virtual source connected
        // to every node with zero-weight edges.
        let mut dist = vec![0.0f64; n];

        for _ in 0..n.saturating_sub(1) {
            let mut changed = false;
            for u in 0..n {
                for edge in &self.edges[u] {
                    let d = dist[u] + edge.neg_log_rate;
                    if d < dist[edge.to] {
                        dist[edge.to] = d;
                        changed = true;
                    }
                }
            }
            if !changed {
                return false;
            }
        }

        for u in 0..n {
            for edge in &self.edges[u] {
                if dist[u] + edge.neg_log_rate < dist[edge.to] {
                    return true;
                }
            }
        }
        false
    }

    fn node(&mut self, token: AccountId) -> usize {
        if let Some(&idx) = self.token_index.get(&token) {
            return idx;
        }
        let idx = self.tokens.len();
        self.tokens.push(token);
        self.token_index.insert(token, idx);
        self.edges.push(Vec::new());
        idx
    }
}

impl Default for PriceGraph {
    fn default() -> Self {
        Self::new()
    }
}
