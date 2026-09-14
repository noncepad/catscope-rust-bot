#[cfg(target_arch = "wasm32")]
use core::arch::wasm32::*;
use std::collections::{HashSet, VecDeque};

use solana_sdk::signature::Signature;

#[derive(Debug, Clone)]
pub struct ArbitrageCycle {
    pub path: Vec<usize>,
    /// Fixed-point, `SCALE_1E8`-scaled -- same representation as
    /// `dist[]` in `detect_negative_cycle`, which this is read directly
    /// from (`reconstruct_cycle`). Callers converting back to a real
    /// yield (e.g. `FundingArbEngine::evaluate_arbitrage_opportunities`)
    /// divide by `SCALE_1E8` before `.exp()`.
    pub total_log_weight: i64,
}

/// 128-bit aligned container holding 2 x f64 edge weights (-ln(rate))
/// and their target node IDs for 2-lane WASM SIMD processing.
#[derive(Clone, Debug)]
pub struct EdgeChunk2 {
    pub weights: [i64; 2], // Fixed-point weights (e.g., scaled by 1e8)
    pub targets: [usize; 2],
}
/// A WASM-native SIMD-accelerated Shortest Path Faster Algorithm (SPFA) graph.
pub struct FinancialGraph {
    num_nodes: usize,
    simd_adj: Vec<Vec<EdgeChunk2>>,
    /// Fixed-point, `SCALE_1E8`-scaled weights -- same representation as
    /// `EdgeChunk2::weights`, so `detect_negative_cycle`'s scalar
    /// fallback loop can compare directly against `dist[]` without an
    /// epsilon buffer, same as the SIMD path.
    scalar_adj: Vec<Vec<(usize, i64)>>,
}
pub const SCALE_1E8: f64 = 100_000_000.0;
impl FinancialGraph {
    pub fn new(num_nodes: usize) -> Self {
        Self {
            num_nodes,
            simd_adj: vec![Vec::new(); num_nodes],
            scalar_adj: vec![Vec::new(); num_nodes],
        }
    }

    /// Add two parallel or outgoing edges simultaneously into a 128-bit WASM v128 SIMD register.
    /// `e1.1`/`e2.1` are raw spot rates, converted to fixed-point
    /// `-ln(rate) * SCALE_1E8` log-weights here -- same conversion
    /// `update_spot_edge_pair` already does for an existing chunk.
    pub fn add_edge_pair(&mut self, from: usize, e1: (usize, f64), e2: (usize, f64)) {
        let w1 = (-e1.1.ln() * SCALE_1E8).round() as i64;
        let w2 = (-e2.1.ln() * SCALE_1E8).round() as i64;
        self.simd_adj[from].push(EdgeChunk2 {
            weights: [w1, w2],
            targets: [e1.0, e2.0],
        });
    }
    /// Add a single residual edge for nodes with an odd number of outgoing edges.
    /// `weight` is already a fixed-point `SCALE_1E8`-scaled log-weight
    /// (e.g. from `calc_perp_log_weight`/`calc_lending_log_weight`), same
    /// representation `simd_adj` stores -- not a raw rate, unlike
    /// `add_edge_pair`'s `f64` params.
    pub fn add_single_edge(&mut self, from: usize, to: usize, weight: i64) {
        self.scalar_adj[from].push((to, weight));
    }
    #[cfg(target_arch = "wasm32")]
    #[inline(always)]
    fn relax_chunk_simd(&self, dist_u: i64, chunk: &EdgeChunk2, dist: &[i64]) -> (u8, [i64; 2]) {
        unsafe {
            // 1. Broadcast i64 dist_u to both lanes
            let u_vec = i64x2_splat(dist_u);

            // 2. Load 2 x i64 weights
            let w_vec = v128_load(chunk.weights.as_ptr() as *const v128);

            // 3. Load 2 x i64 target distances
            let target_dists = [dist[chunk.targets[0]], dist[chunk.targets[1]]];
            let t_vec = v128_load(target_dists.as_ptr() as *const v128);

            // 4. INTEGER ADDITION (No floating-point rounding errors)
            let cand_vec = i64x2_add(u_vec, w_vec);

            // 5. INTEGER COMPARISON: cand_vec < t_vec
            // Note: Fixed-point exact equality means we NO LONGER NEED epsilon buffers!
            let mask = i64x2_lt(cand_vec, t_vec);

            // 6. Extract mask bits
            let byte_mask = u8x16_bitmask(mask);
            let lane0_improved = (byte_mask & 0x0001) != 0;
            let lane1_improved = (byte_mask & 0x0100) != 0;
            let mask_bits = (lane0_improved as u8) | ((lane1_improved as u8) << 1);

            let mut candidates = [0i64; 2];
            v128_store(candidates.as_mut_ptr() as *mut v128, cand_vec);

            (mask_bits, candidates)
        }
    }
    /// Portable scalar equivalent of the `wasm32` SIMD version above --
    /// same integer arithmetic, same comparison, just unrolled instead of
    /// vectorized, so behavior is identical (not an approximation). Lets
    /// this module compile and be unit-tested on a native target, since
    /// the real `wasm32-wasip2` build (this repo's actual target) can't
    /// currently run `cargo test` at all (a separate, pre-existing
    /// tokio-feature conflict unrelated to this file).
    #[cfg(not(target_arch = "wasm32"))]
    #[inline(always)]
    fn relax_chunk_simd(&self, dist_u: i64, chunk: &EdgeChunk2, dist: &[i64]) -> (u8, [i64; 2]) {
        let mut candidates = [0i64; 2];
        let mut mask_bits = 0u8;
        for lane in 0..2 {
            let cand = dist_u + chunk.weights[lane];
            candidates[lane] = cand;
            if cand < dist[chunk.targets[lane]] {
                mask_bits |= 1 << lane;
            }
        }
        (mask_bits, candidates)
    }
    /// Evaluates 2 spot edge relaxations simultaneously using 64-bit Fixed-Point WASM SIMD.
    #[cfg(target_arch = "wasm32")]
    #[inline(always)]
    fn relax_chunk_simd_fixed(
        &self,
        dist_u: i64,
        chunk: &EdgeChunk2,
        dist: &[i64],
    ) -> (u8, [i64; 2]) {
        unsafe {
            // 1. Broadcast dist_u to both 64-bit lanes of the 128-bit WASM register
            let u_vec = i64x2_splat(dist_u);

            // 2. Load 2 x i64 fixed-point weights
            let w_vec = v128_load(chunk.weights.as_ptr() as *const v128);

            // 3. Load target distances
            let target_dists = [dist[chunk.targets[0]], dist[chunk.targets[1]]];
            let t_vec = v128_load(target_dists.as_ptr() as *const v128);

            // 4. WASM SIMD Integer Addition: cand = dist[u] + weight
            let cand_vec = i64x2_add(u_vec, w_vec);

            // 5. WASM SIMD Integer Comparison: cand < target
            // Fixed-point eliminates float drift, making comparisons exact!
            let mask = i64x2_lt(cand_vec, t_vec);

            // 6. Extract active lane bitmask
            let byte_mask = u8x16_bitmask(mask);
            let lane0_improved = (byte_mask & 0x0001) != 0;
            let lane1_improved = (byte_mask & 0x0100) != 0;

            let mask_bits = (lane0_improved as u8) | ((lane1_improved as u8) << 1);

            let mut candidates = [0i64; 2];
            v128_store(candidates.as_mut_ptr() as *mut v128, cand_vec);

            (mask_bits, candidates)
        }
    }
    /// Portable scalar equivalent -- see `relax_chunk_simd`'s
    /// `not(target_arch = "wasm32")` doc comment above.
    #[cfg(not(target_arch = "wasm32"))]
    #[inline(always)]
    fn relax_chunk_simd_fixed(
        &self,
        dist_u: i64,
        chunk: &EdgeChunk2,
        dist: &[i64],
    ) -> (u8, [i64; 2]) {
        self.relax_chunk_simd(dist_u, chunk, dist)
    }
    /// SPFA Negative Cycle Detector optimized for WebAssembly runtimes with SIMD-128
    pub fn detect_negative_cycle(&self) -> Option<ArbitrageCycle> {
        self.detect_negative_cycle_excluding(&HashSet::new())
    }

    /// Same algorithm as `detect_negative_cycle`, skipping any edge whose
    /// `(from, to)` pair is in `excluded` -- a filtered version of the
    /// same relaxation, not a different one. The SIMD chunk call itself
    /// is untouched (still computes both lanes for real); exclusion is
    /// applied afterward, discarding an excluded lane's already-computed
    /// result rather than trying to skip it mid-SIMD-call. Used by
    /// `find_candidate_cycles` to find multiple distinct cycles by
    /// excluding a previously-found cycle's edges and re-running.
    fn detect_negative_cycle_excluding(&self, excluded: &HashSet<(usize, usize)>) -> Option<ArbitrageCycle> {
        let mut dist = vec![0i64; self.num_nodes];
        let mut parent = vec![usize::MAX; self.num_nodes];
        let mut relax_count = vec![0usize; self.num_nodes];
        let mut in_queue = vec![true; self.num_nodes];

        let mut q: VecDeque<usize> = (0..self.num_nodes).collect();
        let mut total_relaxations = 0;
        let cycle_check_interval = self.num_nodes;

        while let Some(u) = q.pop_front() {
            in_queue[u] = false;
            let dist_u = dist[u];

            // --- WASM 128-BIT VECTORIZED RELAXATION LOOP ---
            for chunk in &self.simd_adj[u] {
                let (mask_bits, candidates) = self.relax_chunk_simd(dist_u, chunk, &dist);

                if mask_bits != 0 {
                    for lane in 0..2 {
                        if (mask_bits & (1 << lane)) != 0 {
                            let v = chunk.targets[lane];
                            if excluded.contains(&(u, v)) {
                                continue;
                            }
                            let new_d = candidates[lane];

                            dist[v] = new_d;
                            parent[v] = u;

                            if !in_queue[v] {
                                // Small Label First (SLF) Queue Optimization
                                if let Some(&front) = q.front() {
                                    if dist[v] < dist[front] {
                                        q.push_front(v);
                                    } else {
                                        q.push_back(v);
                                    }
                                } else {
                                    q.push_back(v);
                                }
                                in_queue[v] = true;
                            }
                            relax_count[v] += 1;
                            total_relaxations += 1;

                            if relax_count[v] >= self.num_nodes {
                                return Some(self.reconstruct_cycle(v, &parent, &dist));
                            }
                        }
                    }

                    // Periodic tree parent check for early termination before node relaxes N times
                    if total_relaxations % cycle_check_interval == 0 {
                        if let Some(cycle) = self.check_parent_cycle(u, &parent, &dist) {
                            return Some(cycle);
                        }
                    }
                }
            }

            // --- SCALAR FALLBACK LOOP FOR RESIDUAL EDGES ---
            for &(v, weight) in &self.scalar_adj[u] {
                if excluded.contains(&(u, v)) {
                    continue;
                }
                let new_dist = dist_u + weight;
                // Fixed-point exact equality -- no epsilon buffer needed,
                // same as the SIMD path's i64x2_lt comparison above.
                if new_dist < dist[v] {
                    dist[v] = new_dist;
                    parent[v] = u;

                    if !in_queue[v] {
                        if let Some(&front) = q.front() {
                            if dist[v] < dist[front] {
                                q.push_front(v);
                            } else {
                                q.push_back(v);
                            }
                        } else {
                            q.push_back(v);
                        }
                        in_queue[v] = true;
                    }

                    relax_count[v] += 1;
                    total_relaxations += 1;

                    if relax_count[v] >= self.num_nodes {
                        return Some(self.reconstruct_cycle(v, &parent, &dist));
                    }
                }
            }
        }

        None
    }

    /// Find up to `max_candidates` distinct negative cycles, sorted by
    /// weight (most negative / most profitable first). Needed because a
    /// self-loop (or any short cycle) resolves almost immediately once
    /// negative -- plain `detect_negative_cycle` returns the *first* one
    /// found and stops, which would silently surface only one asset's
    /// opportunity per check when the graph has several independent
    /// negative cycles (e.g. one self-loop per asset). Each pass
    /// excludes every edge of the previously-found cycle (not its
    /// nodes -- excluding nodes would eliminate every other cycle
    /// sharing a hub node) and reruns on the reduced graph.
    pub fn find_candidate_cycles(&self, max_candidates: usize) -> Vec<ArbitrageCycle> {
        let mut excluded: HashSet<(usize, usize)> = HashSet::new();
        let mut candidates = Vec::new();
        while candidates.len() < max_candidates {
            let Some(cycle) = self.detect_negative_cycle_excluding(&excluded) else { break };
            for window in cycle.path.windows(2) {
                excluded.insert((window[0], window[1]));
            }
            candidates.push(cycle);
        }
        candidates.sort_by_key(|c| c.total_log_weight);
        candidates
    }

    fn check_parent_cycle(
        &self,
        start_node: usize,
        parent: &[usize],
        dist: &[i64],
    ) -> Option<ArbitrageCycle> {
        let mut visited = vec![false; self.num_nodes];
        let mut curr = start_node;

        while curr != usize::MAX {
            if visited[curr] {
                return Some(self.reconstruct_cycle(curr, parent, dist));
            }
            visited[curr] = true;
            curr = parent[curr];
        }
        None
    }

    fn reconstruct_cycle(
        &self,
        cycle_node: usize,
        parent: &[usize],
        dist: &[i64],
    ) -> ArbitrageCycle {
        let mut visited = vec![false; self.num_nodes];
        let mut curr = cycle_node;

        // Trace back to ensure landing inside the loop
        while !visited[curr] {
            visited[curr] = true;
            curr = parent[curr];
        }

        let entry = curr;
        let mut path = vec![entry];
        curr = parent[entry];

        while curr != entry {
            path.push(curr);
            curr = parent[curr];
        }
        path.push(entry);
        path.reverse();

        ArbitrageCycle {
            path: path.clone(),
            total_log_weight: dist[entry],
        }
    }
    /// Update an existing edge pair with fresh Spot Market rates or VWAP execution prices.
    pub fn update_spot_edge_pair(
        &mut self,
        from_node: usize,
        chunk_idx: usize,
        spot_rate_1: f64,
        spot_rate_2: f64,
    ) {
        // Convert rates to -ln(rate) scaled by 1e8 fixed-point integer
        let w1 = (-spot_rate_1.ln() * SCALE_1E8).round() as i64;
        let w2 = (-spot_rate_2.ln() * SCALE_1E8).round() as i64;

        if let Some(chunk) = self.simd_adj[from_node].get_mut(chunk_idx) {
            chunk.weights = [w1, w2];
        }
    }
    /// Convert a Perp Market State into an effective log-weight scaled to i64 (1e8)
    pub fn calc_perp_log_weight(perp: &PerpMarketState, holding_hours: f64) -> i64 {
        // Effective price accounting for cumulative funding over the holding horizon
        let funding_impact = perp.hourly_funding_rate * holding_hours;

        let effective_price = match perp.side {
            PositionSide::Long => perp.mark_price * (1.0 + funding_impact), // Pays funding if positive
            PositionSide::Short => perp.mark_price * (1.0 - funding_impact), // Receives funding if positive
        };

        // Convert to fixed-point integer log-weight: -ln(effective_price) * 1e8
        (-effective_price.ln() * SCALE_1E8).round() as i64
    }

    /// Compute fixed-point -ln(effective_rate) * 1e8 for a Lending edge (A -> B)
    pub fn calc_lending_log_weight(market: &LendingMarketState, holding_hours: f64) -> i64 {
        let years = holding_hours / 8760.0; // 365 days * 24 hrs

        // Net interest impact = Earned Supply Interest - Paid Borrow Interest
        let interest_factor = 1.0 + (market.supply_apy * years) - (market.borrow_apy * years);

        // Effective conversion rate of collateral value to borrowed value
        let base_rate = (market.price_a / market.price_b) * market.ltv;
        let effective_rate = base_rate * interest_factor;

        // Convert to fixed-point integer log-weight
        (-effective_rate.ln() * SCALE_1E8 as f64).round() as i64
    }

    /// Calculate the fixed-point log weight for a multi-layer yield edge
    pub fn calculate_yield_edge_weight(
        edge_type: EdgeType,
        holding_hours: f64,
        price_ratio: f64, // (Price To / Price From)
    ) -> i64 {
        let years = holding_hours / 8760.0;

        let effective_rate = match edge_type {
            EdgeType::SpotSwap { fee_tier } => price_ratio * (1.0 - fee_tier),
            EdgeType::PerpFunding {
                hourly_funding,
                is_short,
            } => {
                // Short pays/receives opposite of Long
                let funding_factor = if is_short {
                    -hourly_funding
                } else {
                    hourly_funding
                };
                price_ratio * (1.0 + funding_factor * holding_hours)
            }
            EdgeType::LendingBorrow {
                borrow_apy,
                max_ltv,
            } => {
                let interest_cost = borrow_apy * years;
                price_ratio * max_ltv * (1.0 - interest_cost)
            }
            EdgeType::LendingSupply { supply_apy } => {
                let interest_yield = supply_apy * years;
                price_ratio * (1.0 + interest_yield)
            }
        };

        // Convert effective rate into negative-log fixed-point weight for SIMD cycle detection
        (-effective_rate.ln() * SCALE_1E8).round() as i64
    }
}

/// Unique identifier for market venues (e.g., Binance, Hyperliquid, Bybit).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Venue {
    Binance,
    Hyperliquid,
    Bybit,
    Custom(String),
}

/// Trading pair or market identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Symbol(pub String); // e.g., "SOL-PERP" or "BTC-PERP"

/// Side of the position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionSide {
    Long,
    Short,
}

/// Struct containing market state for a Perp Market Edge
pub struct PerpMarketState {
    pub mark_price: f64,
    pub hourly_funding_rate: f64, // e.g., 0.0001 for +0.01%/hr
    pub side: PositionSide,       // Long or Short
}

#[derive(Debug, Clone)]
pub struct PerpPosition {
    // --- Identification ---
    pub venue: Venue,
    pub symbol: Symbol,

    // --- Position State ---
    pub side: PositionSide,
    /// Absolute size of the position in base units (e.g., 10.5 SOL).
    pub size: f64,
    /// Volume-weighted average entry price (VWAP) across all execution fills.
    pub entry_price: f64,

    // --- Settlement & Accounting State ---
    /// Price at the start of the current hourly settlement interval.
    pub settlement_base_price: f64,
    /// Accumulated realized PnL that has been settled into margin balance.
    pub cumulative_realized_pnl: f64,
    /// Accumulated funding payments paid (negative) or received (positive).
    pub cumulative_funding_paid: f64,

    // --- Time & Interval Tracking ---
    /// Settlement interval duration in seconds (e.g., 3600 for 1 hour).
    pub settlement_interval_secs: u64,
    /// Unix timestamp (seconds) of the LAST applied settlement window boundary.
    /// E.g., 1700000400 for 01:00:00 UTC.
    pub last_settlement_timestamp: u64,
}

impl PerpPosition {
    pub fn new_hourly(
        venue: Venue,
        symbol: Symbol,
        side: PositionSide,
        size: f64,
        fill_price: f64,
        now_unix_secs: u64,
    ) -> Self {
        const HOUR_IN_SECS: u64 = 3600;

        // Floor to the top of the current hour (e.g., 14:23:45 -> 14:00:00)
        let current_hour_boundary = (now_unix_secs / HOUR_IN_SECS) * HOUR_IN_SECS;

        Self {
            venue,
            symbol,
            side,
            size,
            entry_price: fill_price,
            settlement_base_price: fill_price,
            cumulative_realized_pnl: 0.0,
            cumulative_funding_paid: 0.0,
            settlement_interval_secs: HOUR_IN_SECS,
            last_settlement_timestamp: current_hour_boundary,
        }
    }
    /// Calculate current unrealized PnL from initial entry price.
    pub fn unrealized_pnl(&self, current_mark_price: f64) -> f64 {
        let price_diff = current_mark_price - self.entry_price;
        match self.side {
            PositionSide::Long => self.size * price_diff,
            PositionSide::Short => self.size * -price_diff,
        }
    }

    /// Calculate PnL accrued during the CURRENT hourly settlement interval.
    pub fn interval_unrealized_pnl(&self, current_mark_price: f64) -> f64 {
        let price_diff = current_mark_price - self.settlement_base_price;
        match self.side {
            PositionSide::Long => self.size * price_diff,
            PositionSide::Short => self.size * -price_diff,
        }
    }

    /// Check if an hourly settlement boundary has passed and process it.
    pub fn try_settle_hourly(
        &mut self,
        current_time_secs: u64,
        settlement_price: f64,
    ) -> Option<f64> {
        let next_settlement_time = self.last_settlement_timestamp + self.settlement_interval_secs;

        if current_time_secs >= next_settlement_time {
            // Calculate PnL for the completed hour
            let interval_pnl = self.interval_unrealized_pnl(settlement_price);

            // Move interval PnL into realized account balance
            self.cumulative_realized_pnl += interval_pnl;

            // Reset base price for the new hourly window
            self.settlement_base_price = settlement_price;

            // Advance timestamp to the top of the hour that was just settled
            self.last_settlement_timestamp =
                (current_time_secs / self.settlement_interval_secs) * self.settlement_interval_secs;

            Some(interval_pnl) // Return settled cash amount
        } else {
            None // Hour has not elapsed yet
        }
    }

    /// Apply periodic funding rate (e.g., rate = 0.0001 for 0.01%).
    pub fn apply_funding(&mut self, funding_rate: f64, mark_price: f64) -> f64 {
        let position_value = self.size * mark_price;

        // Longs pay shorts when funding is positive
        let funding_payment = match self.side {
            PositionSide::Long => -(position_value * funding_rate),
            PositionSide::Short => position_value * funding_rate,
        };

        self.cumulative_funding_paid += funding_payment;
        funding_payment
    }
}

/// Representation of a Spot Asset Position using 64-bit Fixed-Point Arithmetic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotPosition {
    // --- Identification ---
    pub venue: Venue,
    pub asset: Symbol,

    // --- Position State ---
    /// Total quantity of the asset held, scaled by 1e8 (e.g., 2.5 BTC = 250_000_000).
    pub amount: i64,

    /// Volume-Weighted Average Price (VWAP) cost basis per unit, scaled by 1e8.
    pub avg_cost_basis: i64,

    // --- Performance Tracking ---
    /// Cumulative PnL realized from selling tokens, scaled by 1e8.
    pub cumulative_realized_pnl: i64,

    /// Total cumulative fee expense paid in quote currency, scaled by 1e8.
    pub cumulative_fees_paid: i64,

    /// Unix timestamp (seconds) when this asset was first acquired.
    pub created_at: u64,

    /// Unix timestamp (seconds) of the last trade execution.
    pub last_updated_at: u64,
}

impl SpotPosition {
    /// Instantiate a new spot position from an initial purchase.
    pub fn new(
        venue: Venue,
        asset: Symbol,
        initial_amount: i64,
        fill_price: i64,
        fee: i64,
        now_unix_secs: u64,
    ) -> Self {
        Self {
            venue,
            asset,
            amount: initial_amount,
            avg_cost_basis: fill_price,
            cumulative_realized_pnl: 0,
            cumulative_fees_paid: fee,
            created_at: now_unix_secs,
            last_updated_at: now_unix_secs,
        }
    }

    /// Process a BUY execution. Updates the Volume-Weighted Average Price (VWAP) cost basis.
    pub fn execute_buy(&mut self, buy_amount: i64, fill_price: i64, fee: i64, now_unix_secs: u64) {
        if buy_amount <= 0 {
            return;
        }

        let current_total_value = (self.amount as i128) * (self.avg_cost_basis as i128);
        let new_trade_value = (buy_amount as i128) * (fill_price as i128);

        let new_total_amount = self.amount + buy_amount;

        // Calculate new VWAP: (Old Value + New Value) / Total Amount
        self.avg_cost_basis =
            ((current_total_value + new_trade_value) / (new_total_amount as i128)) as i64;
        self.amount = new_total_amount;
        self.cumulative_fees_paid += fee;
        self.last_updated_at = now_unix_secs;
    }

    /// Process a SELL execution. Calculates and realizes PnL based on cost basis.
    pub fn execute_sell(
        &mut self,
        sell_amount: i64,
        fill_price: i64,
        fee: i64,
        now_unix_secs: u64,
    ) -> Result<i64, &'static str> {
        if sell_amount <= 0 {
            return Err("Sell amount must be positive");
        }
        if sell_amount > self.amount {
            return Err("Insufficient spot balance to sell");
        }

        // Realized PnL per unit = (Sell Price - Cost Basis)
        let price_diff = fill_price - self.avg_cost_basis;

        // Total Realized PnL = (Sell Amount * Price Diff) / 1e8
        let trade_realized_pnl =
            ((sell_amount as i128 * price_diff as i128) / SCALE_1E8 as i128) as i64;

        self.amount -= sell_amount;
        self.cumulative_realized_pnl += trade_realized_pnl;
        self.cumulative_fees_paid += fee;
        self.last_updated_at = now_unix_secs;

        // Reset cost basis to zero if position is fully closed
        if self.amount == 0 {
            self.avg_cost_basis = 0;
        }

        Ok(trade_realized_pnl)
    }

    /// Calculate Unrealized PnL based on the current market mark price.
    pub fn unrealized_pnl(&self, current_mark_price: i64) -> i64 {
        let price_diff = current_mark_price - self.avg_cost_basis;
        ((self.amount as i128 * price_diff as i128) / SCALE_1E8 as i128) as i64
    }

    /// Calculate total position market value in quote currency.
    pub fn market_value(&self, current_mark_price: i64) -> i64 {
        ((self.amount as i128 * current_mark_price as i128) / SCALE_1E8 as i128) as i64
    }
}

pub struct SpotArbitrageSystem {
    pub graph: FinancialGraph,
    pub positions: Vec<SpotPosition>, // Map node ID -> Spot Position
}

impl SpotArbitrageSystem {
    /// Applies a spot trade execution and updates the WASM SIMD Graph weights.
    pub fn on_spot_trade_executed(
        &mut self,
        from_node: usize,
        to_node: usize,
        chunk_idx: usize,
        lane: usize,
        fill_price_rate: f64,
        traded_amount: i64,
        fee: i64,
        now_secs: u64,
    ) {
        // 1. Update underlying Spot Position accounting (VWAP cost basis, balances)
        if let Some(pos) = self.positions.get_mut(to_node) {
            pos.execute_buy(
                traded_amount,
                (fill_price_rate * SCALE_1E8) as i64,
                fee,
                now_secs,
            );
        }

        // 2. Convert new rate into log weight: -ln(rate) * 1e8
        let new_fixed_weight = (-fill_price_rate.ln() * SCALE_1E8).round() as i64;

        // 3. Directly mutate the WASM SIMD Graph edge chunk
        let chunk = &mut self.graph.simd_adj[from_node][chunk_idx];
        chunk.weights[lane] = new_fixed_weight;

        // 4. Trigger cycle detection with the updated WASM SIMD graph state
        if let Some(cycle) = self.graph.detect_negative_cycle() {
            println!(
                "Arbitrage Cycle Detected across Spot Positions: {:?}",
                cycle.path
            );
        }
    }
}

pub struct PerpArbitrageSystem {
    pub graph: FinancialGraph,
    pub positions: Vec<PerpPosition>, // Active Perp Positions mapped by node ID
    pub target_holding_hours: f64,    // Expected duration to hold the arb cycle
}

impl PerpArbitrageSystem {
    /// Called whenever a orderbook tick OR funding rate update arrives for a Perp market
    pub fn on_perp_market_update(
        &mut self,
        from_node: usize,
        to_node: usize,
        chunk_idx: usize,
        lane: usize,
        updated_perp: &PerpMarketState,
        now_secs: u64,
    ) {
        // 1. Process periodic hourly funding settlements on open perp positions
        if let Some(pos) = self.positions.get_mut(to_node) {
            pos.try_settle_hourly(now_secs, updated_perp.mark_price);
            pos.apply_funding(updated_perp.hourly_funding_rate, updated_perp.mark_price);
        }

        // 2. Compute the new fixed-point net log weight (-ln(price_eff) * 1e8)
        let new_fixed_weight =
            FinancialGraph::calc_perp_log_weight(updated_perp, self.target_holding_hours);

        // 3. Mutate the 128-bit SIMD chunk in-place
        let chunk = &mut self.graph.simd_adj[from_node][chunk_idx];
        chunk.weights[lane] = new_fixed_weight;

        // 4. Run the fixed-point WASM SIMD negative cycle detector (using i64x2_add & i64x2_lt)
        if let Some(cycle) = self.graph.detect_negative_cycle() {
            println!("Perp Arbitrage Cycle Found (Net of Funding)!");
            println!("Path: {:?}", cycle.path);
            println!("Total Log Weight: {}", cycle.total_log_weight);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LendingPosition {
    pub venue: String,            // e.g., "Solend", "Marginfi"
    pub collateral_asset: String, // Token A (e.g., "SOL")
    pub borrowed_asset: String,   // Token B (e.g., "USDC")

    /// Scaled by 1e8 (e.g., 100 SOL = 10_000_000_000)
    pub collateral_amount: i64,
    /// Scaled by 1e8 (e.g., 5,000 USDC = 500_000_000_000)
    pub borrowed_amount: i64,

    /// Protocol Max Loan-To-Value ratio scaled by 1e8 (e.g., 0.75 = 75_000_000)
    pub max_ltv: i64,
    /// Liquidation threshold scaled by 1e8 (e.g., 0.80 = 80_000_000)
    pub liquidation_threshold: i64,

    /// Annualized Borrow Interest Rate on Token B scaled by 1e8
    pub borrow_apy: i64,
    /// Annualized Supply Yield on Token A scaled by 1e8
    pub supply_apy: i64,
}

impl LendingPosition {
    /// Returns current Loan-To-Value ratio (in 1e8 fixed point)
    pub fn current_ltv(&self, price_a: i64, price_b: i64) -> i64 {
        if self.collateral_amount == 0 {
            return 0;
        }

        let collateral_val = (self.collateral_amount as i128 * price_a as i128) / SCALE_1E8 as i128;
        let borrow_val = (self.borrowed_amount as i128 * price_b as i128) / SCALE_1E8 as i128;

        ((borrow_val * SCALE_1E8 as i128) / collateral_val) as i64
    }

    /// Health Factor = (Collateral Value * Liquidation Threshold) / Borrowed Value
    /// Values < 1.0 (1e8) mean the position is eligible for liquidation.
    pub fn health_factor(&self, price_a: i64, price_b: i64) -> i64 {
        if self.borrowed_amount == 0 {
            return i64::MAX;
        }

        let collateral_val = (self.collateral_amount as i128 * price_a as i128) / SCALE_1E8 as i128;
        let liquidation_val =
            (collateral_val * self.liquidation_threshold as i128) / SCALE_1E8 as i128;
        let borrow_val = (self.borrowed_amount as i128 * price_b as i128) / SCALE_1E8 as i128;

        ((liquidation_val * SCALE_1E8 as i128) / borrow_val) as i64
    }
}

pub struct LendingMarketState {
    pub price_a: f64,    // Collateral Price
    pub price_b: f64,    // Borrowed Asset Price
    pub ltv: f64,        // e.g., 0.75
    pub supply_apy: f64, // e.g., 0.04 (4%)
    pub borrow_apy: f64, // e.g., 0.07 (7%)
}

/// Represents edge conversion types across layers
pub enum EdgeType {
    SpotSwap { fee_tier: f64 },
    PerpFunding { hourly_funding: f64, is_short: bool },
    LendingBorrow { borrow_apy: f64, max_ltv: f64 },
    LendingSupply { supply_apy: f64 },
}

/// Thread-safe, atomically configurable risk tolerances.
#[derive(Debug)]
pub struct DynamicRiskConfig {
    /// Minimum allowed margin ratio buffer before rebalance (e.g., 1.50 = 150_000_000).
    /// `i64`, not `u64` -- compared directly against `AccountMarginState::margin_ratio()`'s
    /// `i64` return, same fixed-point representation as every other
    /// threshold field here.
    pub safety_buffer_threshold: i64,

    /// Emergency unwinding threshold (e.g., 1.15 = 115_000_000)
    pub emergency_unwind_threshold: i64,

    /// Max leverage cap allowed per venue (e.g., 5.0x = 500_000_000)
    pub max_leverage_cap: i64,

    /// Emergency global kill switch flag to halt trading / auto-unwind
    pub global_kill_switch: bool,
}
impl Default for DynamicRiskConfig {
    fn default() -> Self {
        Self {
            safety_buffer_threshold: 150_000_000,    // 1.50x
            emergency_unwind_threshold: 115_000_000, // 1.15x
            max_leverage_cap: 500_000_000,           // 5.0x
            global_kill_switch: false,
        }
    }
}

#[derive(Debug)]
pub enum TradeAction {
    BuySpot {
        asset: String,
        amount: i64,
    },
    SellSpot {
        asset: String,
        amount: i64,
    },
    OpenPerpShort {
        symbol: String,
        size: i64,
    },
    OpenPerpLong {
        symbol: String,
        size: i64,
    },
    DepositCollateral {
        venue: String,
        asset: String,
        amount: i64,
    },
    BorrowAsset {
        venue: String,
        asset: String,
        amount: i64,
    },
}

/// Store a set of instructions that are pending in order to calculate pending position changes.
pub struct PendingTransaction {
    pub signature: Signature,
    pub instruction: Vec<PendingInstruction>,
}
pub struct PendingInstruction {
    pub long: Position,
    pub short: Position,
}
pub enum Position {
    Spot(SpotPosition),
    Perp(PerpPosition),
}

#[derive(Debug)]
pub struct ArbitrageExecutionPlan {
    pub actions: Vec<TradeAction>,
    pub expected_net_yield_bps: i64,
    pub holding_horizon_hours: f64,
    pub delta_neutral: bool,
}

pub struct FundingArbEngine {
    pub cvmm: CrossVenueMarginManager,
    pub graph: FinancialGraph,
    pub spot_positions: Vec<SpotPosition>,
    pub perp_positions: Vec<PerpPosition>,
    pub lending_positions: Vec<LendingPosition>,
}

impl FundingArbEngine {
    pub fn new(cvmm: CrossVenueMarginManager, num_nodes: usize) -> Self {
        Self {
            cvmm,
            graph: FinancialGraph::new(num_nodes),
            spot_positions: Vec::with_capacity(num_nodes),
            perp_positions: Vec::with_capacity(num_nodes),
            lending_positions: Vec::with_capacity(num_nodes),
        }
    }
    /// Evaluates WASM SIMD graph and produces a structured execution plan
    pub fn evaluate_arbitrage_opportunities(
        &self,
        holding_hours: f64,
    ) -> Option<ArbitrageExecutionPlan> {
        // 1. Run WASM i64x2 SIMD SPFA negative cycle detector
        let cycle = self.graph.detect_negative_cycle()?;

        // 2. Decode the node path into discrete market actions
        let mut actions = Vec::new();
        let mut delta_check = 0.0f64;

        for window in cycle.path.windows(2) {
            let u = window[0];
            let v = window[1];

            // Map graph node indices to explicit market actions
            let action = self.map_nodes_to_action(u, v);

            // Track net delta exposure
            match &action {
                TradeAction::BuySpot { .. } | TradeAction::OpenPerpLong { .. } => {
                    delta_check += 1.0
                }
                TradeAction::SellSpot { .. } | TradeAction::OpenPerpShort { .. } => {
                    delta_check -= 1.0
                }
                _ => {}
            }

            actions.push(action);
        }

        // 3. Convert total log weight back into net basis points (BPS)
        let total_yield_factor = (-cycle.total_log_weight as f64 / SCALE_1E8).exp() - 1.0;
        let expected_net_yield_bps = (total_yield_factor * 10000.0) as i64;

        Some(ArbitrageExecutionPlan {
            actions,
            expected_net_yield_bps,
            holding_horizon_hours: holding_hours,
            delta_neutral: delta_check.abs() < 1e-4,
        })
    }

    fn map_nodes_to_action(&self, _u: usize, _v: usize) -> TradeAction {
        // Logic mapping node pair IDs (e.g., Node(Spot_SOL) -> Node(Perp_SOL_Short))
        // to explicit TradeAction variants
        todo!("Map node indices to market actions")
    }
}

#[derive(Debug, Clone)]
pub struct AccountMarginState {
    pub venue_id: String,

    /// Total raw balance in quote currency (e.g., USDC), scaled by 1e8
    pub cash_balance: i64,

    /// Total value of deposited collateral (unweighted), scaled by 1e8
    pub collateral_market_value: i64,

    /// Weighted collateral value after applying venue haircuts (e.g. SOL at 80% weight = 0.8 * Value)
    pub effective_collateral_value: i64,

    /// Total initial margin required for open positions, scaled by 1e8
    pub initial_margin_required: i64,

    /// Maintenance margin threshold. If Equity < Maintenance Margin, liquidation triggers!
    pub maintenance_margin_required: i64,

    /// Current unrealized PnL on open derivative positions, scaled by 1e8
    pub unrealized_pnl: i64,
}

impl AccountMarginState {
    /// Calculate current total account equity: Cash + Effective Collateral + Unrealized PnL
    pub fn total_equity(&self) -> i64 {
        self.cash_balance + self.effective_collateral_value + self.unrealized_pnl
    }

    /// Calculate free available margin for placing new trades
    pub fn available_margin(&self) -> i64 {
        let equity = self.total_equity();
        if equity > self.initial_margin_required {
            equity - self.initial_margin_required
        } else {
            0
        }
    }

    /// Calculate Margin Ratio = Total Equity / Maintenance Margin Required
    /// A value of 1.0 (1e8) means the account is on the verge of liquidation.
    pub fn margin_ratio(&self) -> i64 {
        if self.maintenance_margin_required == 0 {
            return i64::MAX;
        }
        ((self.total_equity() as i128 * SCALE_1E8 as i128)
            / self.maintenance_margin_required as i128) as i64
    }
}

pub struct CrossVenueMarginManager {
    pub config: DynamicRiskConfig,
    pub accounts: Vec<AccountMarginState>,
}

impl CrossVenueMarginManager {
    pub fn new(config: DynamicRiskConfig) -> Self {
        Self {
            config,
            accounts: Vec::with_capacity(1024),
        }
    }

    /// Update the risk tolerance configuration.
    pub fn update_config(&mut self, config: DynamicRiskConfig) {
        self.config = config;
    }

    /// Calculate the exact Mark Price where a Short Perp leg gets liquidated
    pub fn calc_short_perp_liquidation_price(
        entry_price: i64,
        position_size: i64,
        account_equity: i64,
        maint_margin_rate: i64, // e.g., 0.05 (5%) = 5_000_000
    ) -> i64 {
        if position_size == 0 {
            return i64::MAX;
        }

        let size_i128 = position_size as i128;
        let entry_i128 = entry_price as i128;
        let equity_i128 = account_equity as i128;
        let rate_i128 = maint_margin_rate as i128;

        // Position Value at Entry = Size * Entry Price / 1e8
        let pos_value = (size_i128 * entry_i128) / SCALE_1E8 as i128;
        let numerator = (equity_i128 + pos_value) * SCALE_1E8 as i128;
        let denominator = (size_i128 * (SCALE_1E8 as i128 + rate_i128)) / SCALE_1E8 as i128;

        (numerator / denominator) as i64
    }

    /// Evaluates cross-venue health and checks if collateral rebalancing is needed
    pub fn check_rebalance_triggers(&self) -> Vec<RebalanceInstruction> {
        let mut instructions = Vec::new();

        for acc in &self.accounts {
            let ratio = acc.margin_ratio();

            // Trigger rebalance if margin ratio drops below our safety threshold (e.g., 1.5x)
            if ratio < self.config.safety_buffer_threshold {
                let required_capital = acc.maintenance_margin_required - acc.total_equity();

                instructions.push(RebalanceInstruction {
                    target_venue: acc.venue_id.clone(),
                    required_amount: required_capital,
                    urgency: if ratio < 120_000_000 {
                        Urgency::CRITICAL
                    } else {
                        Urgency::MEDIUM
                    },
                });
            }
        }

        instructions
    }
}

#[derive(Debug)]
pub enum Urgency {
    MEDIUM,
    CRITICAL,
}

#[derive(Debug)]
pub struct RebalanceInstruction {
    pub target_venue: String,
    pub required_amount: i64,
    pub urgency: Urgency,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `reconstruct_cycle` sets `total_log_weight` to `dist[entry]` --
    /// a live snapshot of Bellman-Ford's `dist[]` at the moment some
    /// node's `relax_count` first hits the `num_nodes` threshold, not
    /// a clean single traversal of the cycle's raw edge weight (a
    /// self-loop keeps re-relaxing itself every round, so `dist[]`
    /// accumulates `raw_weight` multiple times before the threshold
    /// fires -- confirmed by running these tests before writing exact
    /// assertions, e.g. a single `-42` self-loop in a 2-node graph
    /// reads back as `dist[]` accumulated to `-84`). This is a
    /// pre-existing property of `detect_negative_cycle`/
    /// `reconstruct_cycle`, unrelated to `find_candidate_cycles`'
    /// exclusion logic, so these tests assert the invariants that
    /// actually matter to callers -- sign, cycle count, path shape,
    /// relative ordering -- rather than a magic accumulated number.

    /// Three independent negative self-loops (nodes 0, 1, 2) plus a
    /// positive-weight edge (node 3) that must never surface. Uses
    /// `add_single_edge` only, which writes to `scalar_adj` -- the
    /// portable path exercised on every target, not gated behind
    /// `target_arch = "wasm32"`, so this is a real exercise of the
    /// production relaxation logic natively, not an approximation of
    /// it (the SIMD `simd_adj` path is independently verified live,
    /// per this session's smoke test).
    #[test]
    fn find_candidate_cycles_finds_all_independent_self_loops() {
        let mut g = FinancialGraph::new(4);
        g.add_single_edge(0, 0, -100);
        g.add_single_edge(1, 1, -50);
        g.add_single_edge(2, 2, -200);
        g.add_single_edge(3, 3, 10); // positive -- not a negative cycle, must not appear

        let cycles = g.find_candidate_cycles(4);

        assert_eq!(cycles.len(), 3, "expected exactly the 3 negative self-loops, got {cycles:?}");

        let mut nodes: Vec<usize> = cycles.iter().map(|c| c.path[0]).collect();
        nodes.sort();
        assert_eq!(nodes, vec![0, 1, 2], "node 3's positive self-loop must not appear, and each negative one exactly once");

        for cycle in &cycles {
            assert_eq!(cycle.path.len(), 2, "a self-loop's path should be [v, v]");
            assert_eq!(cycle.path[0], cycle.path[1]);
            assert!(cycle.total_log_weight < 0, "a returned cycle must actually be negative: {cycle:?}");
        }
        // Sorted most-negative-first.
        assert!(cycles.windows(2).all(|w| w[0].total_log_weight <= w[1].total_log_weight));
    }

    /// `max_candidates` bounds the search even when more negative
    /// cycles exist in the graph -- callers rely on this to cap
    /// work to the real asset-universe size.
    #[test]
    fn find_candidate_cycles_respects_max_candidates() {
        let mut g = FinancialGraph::new(3);
        g.add_single_edge(0, 0, -10);
        g.add_single_edge(1, 1, -20);
        g.add_single_edge(2, 2, -30);

        let cycles = g.find_candidate_cycles(2);

        assert_eq!(cycles.len(), 2, "3 negative cycles exist but max_candidates=2 must cap the result");
        for cycle in &cycles {
            assert!(cycle.total_log_weight < 0);
        }
        assert!(cycles.windows(2).all(|w| w[0].total_log_weight <= w[1].total_log_weight));
    }

    /// No negative cycles at all -> empty, not a panic or a false
    /// positive.
    #[test]
    fn find_candidate_cycles_empty_graph_returns_none() {
        let mut g = FinancialGraph::new(2);
        g.add_single_edge(0, 1, 5);
        g.add_single_edge(1, 0, 5);

        let cycles = g.find_candidate_cycles(5);

        assert!(cycles.is_empty(), "expected no candidates, got {cycles:?}");
    }

    /// Once a cycle's edges are excluded, `find_candidate_cycles`
    /// doesn't just keep re-finding the same one -- confirms
    /// `detect_negative_cycle_excluding`'s exclusion set actually
    /// changes the graph the next pass searches, not merely a
    /// theoretical bound from `max_candidates` alone (`max_candidates`
    /// here is deliberately larger than the number of real distinct
    /// cycles in the graph).
    #[test]
    fn find_candidate_cycles_excludes_previously_found_edges() {
        let mut g = FinancialGraph::new(2);
        g.add_single_edge(0, 0, -42);

        let cycles = g.find_candidate_cycles(5);

        assert_eq!(cycles.len(), 1, "only one distinct negative cycle exists, exclusion must stop the search, not loop");
        assert!(cycles[0].total_log_weight < 0);
    }
}
