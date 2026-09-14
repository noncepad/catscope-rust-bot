//! Fair-price graph and multi-hop trade router.
//!
//! # PriceGraph — fair-price and arbitrage detection
//!
//! Nodes are token mints (`AccountId`); directed edges are swap routes
//! weighted by `−ln(effective_rate)`, where
//! `effective_rate = spot_price × (1 − fee)`.
//!
//! Bellman-Ford finds the minimum-cost path, which corresponds to the
//! maximum cumulative exchange rate from a reference token to every other
//! reachable token.
//!
//! # TradeRouter — METIS multi-hop routing
//!
//! Finds the best execution path from one token to another through up to
//! `max_hops` Orca pools. Uses Bellman-Ford on log-space edge weights
//! (same metric as PriceGraph) with predecessor tracking to reconstruct
//! the full swap sequence, then cascades a constant-product quote through
//! each hop to estimate the final output.
//!
//! ```rust,ignore
//! let router = TradeRouter::from_orca_pools(&state.m_orca_pool);
//! if let Some(route) = router.route(usdc_id, sol_id, 1_000_000, 3) {
//!     for hop in &route.hops {
//!         // hop.pool_id, hop.input_mint, hop.output_mint, hop.amount_in, hop.amount_out
//!     }
//!     println!("expected out: {}", route.amount_out());
//! }
//! ```

use crate::{
    graph::AccountId,
    trader::dex::{
        orca::OrcaWhirlpool,
        raydium::{amm::RaydiumAmmPool, clmm::RaydiumClmmPool},
    },
    trader::types::DexType,
};
use solana_sdk::clock::Slot;
use std::collections::{HashMap, HashSet};

// ─── PriceGraph ───────────────────────────────────────────────────────────────

struct Edge {
    to: usize,
    /// `−ln(effective_rate)`: minimising this maximises the exchange rate.
    neg_log_rate: f64,
}

/// Token exchange-rate graph for fair-price and arbitrage calculation.
pub struct PriceGraph {
    token_index: HashMap<AccountId, usize>,
    tokens: Vec<AccountId>,
    edges: Vec<Vec<Edge>>,
}

impl PriceGraph {
    pub fn new() -> Self {
        Self {
            token_index: HashMap::new(),
            tokens: Vec::new(),
            edges: Vec::new(),
        }
    }

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
    /// Returns `None` only if `reference` is not in the graph.
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
                prices.insert(self.tokens[idx], (-d).exp());
            }
        }
        Some(prices)
    }

    /// Returns `true` if any negative cycle (arbitrage loop) exists.
    pub fn has_arbitrage(&self) -> bool {
        let n = self.tokens.len();
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

// ─── TradeRouter (METIS multi-hop routing) ───────────────────────────────────
#[derive(Debug)]
struct RouterEdge {
    /// Destination token node index.
    to: usize,
    /// `−ln(spot × (1 − fee))` — minimised by Bellman-Ford.
    neg_log_rate: f64,
    pub pool_id: AccountId,
    pub input_mint: AccountId,
    pub output_mint: AccountId,
    /// Reserve of the input token (raw units). Used for CP-AMM quote.
    reserve_in: u64,
    /// Reserve of the output token (raw units). Used for CP-AMM quote.
    reserve_out: u64,
    /// Pool fee in basis points (e.g. 30 = 0.30%).
    fee_bps: u16,
    /// Which DEX protocol this edge's pool belongs to.
    dex: DexType,
}

/// One swap leg along a route.
#[derive(Debug, Clone)]
pub struct Hop {
    pub pool_id: AccountId,
    pub input_mint: AccountId,
    pub output_mint: AccountId,
    /// Tokens sent into this pool.
    pub amount_in: u64,
    /// Tokens received from this pool (constant-product estimate).
    pub amount_out: u64,
    /// Which DEX protocol this hop's pool belongs to.
    pub dex: DexType,
}

/// Max fraction (basis points of `edge.reserve_in`) of a single pool's
/// own reserve that [`TradeRouter::widest_path`] will trade through it in
/// one hop -- see that function's own doc comment for the real, live
/// incident this fixes (a nearly-drained pool's `cp_quote` output looking
/// like the numerically "best" edge purely as a constant-product-formula
/// artifact, not because it's actually deep or cheap). `2_000` (20%) is a
/// starting value, not calibrated -- same "starting value" honesty as
/// `factor_sizing::MAX_PRICE_IMPACT_BPS`'s own doc comment; this is a
/// coarse, dimensionless filter to keep the pathfinder from being fooled
/// by an obviously-too-small pool, not a substitute for a real
/// price-impact check against the *specific* trade size a caller
/// actually wants (that's `factor_sizing::max_safe_notional`'s job).
const MAX_HOP_POOL_UTILIZATION_BPS: u32 = 2_000;

/// Whether any two hops in a candidate `(node, edge_index)` path (from
/// [`TradeRouter::reconstruct_path`]/[`TradeRouter::reconstruct_cycle`])
/// share the same `pool_id` -- a route that does isn't safe to send as one
/// atomic transaction. Live-confirmed this session: a real Raydium CLMM
/// `SwapV2` panicked on-chain (`already mutably borrowed: BorrowError`)
/// when a `widest_path`-derived route happened to touch the same pool at
/// two non-adjacent hops (the DP that builds these routes has no "already
/// used" tracking -- it's a pure best-value-per-hop-depth relaxation, so
/// nothing prevents this by construction, and live-confirmed to
/// deterministically re-pick the same unsafe route on every retry when
/// reserves haven't moved enough to change the DP's answer). Checked
/// against the raw path (not yet turned into priced [`Hop`]s) so callers
/// can reject a bad candidate and fall back to a different hop count
/// without paying for `cp_quote` on a path they're about to throw away.
fn path_reuses_a_pool(path: &[(usize, usize)], edges: &[Vec<RouterEdge>]) -> bool {
    let mut seen = HashSet::with_capacity(path.len());
    !path.iter().all(|&(u, ei)| seen.insert(edges[u][ei].pool_id))
}

/// Whether `path` touches any token node more than once -- including
/// looping back through `src` itself -- via *distinct* pools, so this
/// catches a real degenerate case [`path_reuses_a_pool`] does not: three
/// distinct pools can still compose a nonsensical route that passes
/// through the same token twice. Live-confirmed this session: for a real
/// sell-direction quote, `widest_path`'s "best" 3-hop path for
/// `mint -> USDC` was actually `mint -> USDC -> X -> USDC`, revisiting
/// USDC mid-route through a pool with only ~$22 of real reserve on one
/// side -- trading through more than half that pool's depth produced a
/// wildly inflated (~58x) quoted output, a `cp_quote` artifact of a
/// near-drained pool, not a real 58x arbitrage. Only meaningful for
/// point-to-point routing ([`TradeRouter::route_slippage_aware`]) --
/// deliberately **not** applied to [`TradeRouter::find_arbitrage_slippage_aware`],
/// which legitimately requires the path to return to `src` (that's the
/// whole point of a cycle).
fn path_revisits_a_node(path: &[(usize, usize)], src: usize, edges: &[Vec<RouterEdge>]) -> bool {
    let mut seen = HashSet::with_capacity(path.len() + 1);
    seen.insert(src);
    !path.iter().all(|&(u, ei)| seen.insert(edges[u][ei].to))
}

/// A complete multi-hop swap route returned by [`TradeRouter::route`].
#[derive(Debug, Clone)]
pub struct Route {
    pub hops: Vec<Hop>,
}

impl Route {
    pub fn token_in(&self) -> AccountId {
        self.hops.first().map(|h| h.input_mint).unwrap_or(0)
    }
    pub fn token_out(&self) -> AccountId {
        self.hops.last().map(|h| h.output_mint).unwrap_or(0)
    }
    /// Final estimated output after all hops.
    pub fn amount_out(&self) -> u64 {
        self.hops.last().map(|h| h.amount_out).unwrap_or(0)
    }
    pub fn n_hops(&self) -> usize {
        self.hops.len()
    }
}

/// A closed arbitrage loop: starts and ends at the same token.
#[derive(Debug, Clone)]
pub struct ArbitrageCycle {
    pub hops: Vec<Hop>,
}

impl ArbitrageCycle {
    /// The token the cycle starts (and ends) with.
    pub fn start_token(&self) -> AccountId {
        self.hops.first().map(|h| h.input_mint).unwrap_or(0)
    }
    pub fn amount_in(&self) -> u64 {
        self.hops.first().map(|h| h.amount_in).unwrap_or(0)
    }
    pub fn amount_out(&self) -> u64 {
        self.hops.last().map(|h| h.amount_out).unwrap_or(0)
    }
    /// Gross profit in raw units of `start_token`.
    pub fn profit_raw(&self) -> u64 {
        self.amount_out().saturating_sub(self.amount_in())
    }
    /// Gross profit in basis points of `amount_in`.
    pub fn profit_bps(&self) -> u64 {
        self.profit_raw().saturating_mul(10_000) / self.amount_in().max(1)
    }
    /// Priority fee in micro-lamports per CU, assuming start_token is SOL (lamports).
    ///
    /// `profit_fraction`: e.g. `0.5` to allocate 50 % of gross profit as fees.
    /// Pass `wallet.cu()` as `total_cu` **after** appending all swap instructions.
    pub fn priority_micro_lamports_per_cu(&self, profit_fraction: f64, total_cu: u32) -> u64 {
        if total_cu == 0 {
            return 0;
        }
        let fee_lamports = (self.profit_raw() as f64 * profit_fraction) as u64;
        fee_lamports.saturating_mul(1_000_000) / total_cu as u64
    }

    /// Net profit in raw lamports (`profit_raw` minus the real cost of
    /// actually sending this transaction) -- **assumes `start_token` is
    /// SOL**, same convention as `priority_micro_lamports_per_cu`; callers
    /// must check `start_token() == mint_sol` themselves before calling
    /// this, since a non-SOL `start_token`'s `profit_raw()` isn't
    /// denominated in lamports and can't be netted against a lamport fee
    /// without a separate price conversion this method deliberately
    /// doesn't attempt.
    ///
    /// `total_cu`: pass `wallet.cu()` **after** appending all swap
    /// instructions (the real, built cost -- not an estimate).
    /// `priority_micro_lamports_per_cu`: the priority fee rate actually
    /// being bid (e.g. `PriorityLevel::Medium.into()`).
    /// `num_signatures`: almost always `1` for this bot's own arbitrage
    /// transactions (single-signer, the bot's own wallet).
    ///
    /// Signed (`i64`, not `u64`): a "profitable" cycle by `profit_raw`
    /// alone can still be a net loss once real transaction costs are
    /// subtracted, especially for a thin cycle whose gross profit is only
    /// a few thousand lamports -- reporting that as a negative number
    /// rather than saturating at 0 is the whole point of this method.
    pub fn net_profit_lamports(
        &self,
        total_cu: u32,
        priority_micro_lamports_per_cu: u64,
        num_signatures: u64,
    ) -> i64 {
        let base_fee = num_signatures.saturating_mul(SOLANA_BASE_FEE_LAMPORTS_PER_SIGNATURE);
        let priority_fee = (total_cu as u64).saturating_mul(priority_micro_lamports_per_cu) / 1_000_000;
        self.profit_raw() as i64 - base_fee as i64 - priority_fee as i64
    }
}

/// Solana's protocol-level base transaction fee, per signature -- fixed
/// at 5,000 lamports network-wide (independent of compute units or
/// priority fee). See `ArbitrageCycle::net_profit_lamports`.
const SOLANA_BASE_FEE_LAMPORTS_PER_SIGNATURE: u64 = 5_000;

/// Multi-hop trade router built from a set of Orca Whirlpool pools.
///
/// Build once per evaluate cycle (or whenever pool states change), then call
/// [`route`](Self::route) for each trade you want to place.
#[derive(Debug)]
pub struct TradeRouter {
    token_index: HashMap<AccountId, usize>,
    tokens: Vec<AccountId>,
    /// Adjacency list: `edges[u]` = outgoing edges from token node `u`.
    edges: Vec<Vec<RouterEdge>>,
    /// Updated by callers (once per commit/low_latency batch) via
    /// `set_current_slot` -- used only to evaluate `m_pool_cooldown`
    /// entries, so staleness here just means a cooldown expires a few
    /// hundred ms later/earlier than intended, not a correctness issue.
    current_slot: Slot,
    /// pool_id -> the slot after which it's eligible for the
    /// slippage-aware search again. Populated by `mark_pool_cooldown`
    /// (called from `arbv1::state.rs` when `planner::reverify_hops`
    /// reports a specific pool's exact quote disagreed badly with this
    /// router's own constant-product approximation -- see that
    /// function's doc comment for the incident that motivated this).
    /// Only consulted by the slippage-aware search
    /// (`widest_path`/`route_slippage_aware`/`find_arbitrage_slippage_aware`)
    /// -- the cheap amount-blind log-space search (`route`/
    /// `find_arbitrage`) is unaffected, since it never builds a `Hop` an
    /// exact quote could disagree with in the first place.
    m_pool_cooldown: HashMap<AccountId, Slot>,
    /// pool_id -> consecutive real `PoolNotReady` failures observed so
    /// far. `PoolNotReady` is deliberately excluded from the normal
    /// one-shot cooldown above (see `note_pool_not_ready`'s own doc
    /// comment for why) -- this tracks the *repeated* case separately,
    /// so a pool that keeps failing this way isn't retried forever.
    m_pool_not_ready_streak: HashMap<AccountId, u32>,
    /// pool_id -> how many times `note_pool_not_ready` has crossed its
    /// threshold for this pool, i.e. how many separate cooldown cycles
    /// it's already been through. Drives `mark_pool_not_ready_cooldown`'s
    /// escalating duration -- see that function's own doc comment.
    m_pool_not_ready_cooldown_cycles: HashMap<AccountId, u32>,
}

impl Default for TradeRouter {
    fn default() -> Self {
        Self::new()
    }
}
pub struct Pool<'a> {
    pub mint_a: &'a AccountId,
    pub mint_b: &'a AccountId,
}
impl TradeRouter {
    pub fn new() -> Self {
        Self {
            token_index: HashMap::new(),
            tokens: Vec::new(),
            edges: Vec::new(),
            current_slot: 0,
            m_pool_cooldown: HashMap::new(),
            m_pool_not_ready_streak: HashMap::new(),
            m_pool_not_ready_cooldown_cycles: HashMap::new(),
        }
    }

    /// How many consecutive real `PoolNotReady` failures on the same pool
    /// before treating it as genuinely stuck rather than "just hasn't
    /// finished subscribing yet" -- live-confirmed (2026-09-07) a real
    /// pool can fail identically for 30+ minutes across multiple process
    /// restarts, well past what "still subscribing" could explain.
    pub const POOL_NOT_READY_COOLDOWN_THRESHOLD: u32 = 5;

    /// Records a real `PoolNotReady` failure for `pool_id`. `PoolNotReady`
    /// is deliberately excluded from the normal one-shot cooldown (see
    /// every caller's own `coolable` comment -- it can be the *only*
    /// viable pool for a route, e.g. a stranded token, where cooling down
    /// on the very first failure would foreclose the only path instead of
    /// giving its subscription a chance to catch up). This tracks
    /// *repeated* failures on the same pool instead: once it's failed
    /// `POOL_NOT_READY_COOLDOWN_THRESHOLD` times, treat it as genuinely
    /// stuck and tell the caller to cool it down like any other real
    /// failure -- returns `true` exactly once per threshold crossed, and
    /// resets the streak so it takes a full threshold's worth of new
    /// failures after the cooldown expires before cooling down again.
    pub fn note_pool_not_ready(&mut self, pool_id: AccountId) -> bool {
        let count = self.m_pool_not_ready_streak.entry(pool_id).or_insert(0);
        *count += 1;
        if *count >= Self::POOL_NOT_READY_COOLDOWN_THRESHOLD {
            self.m_pool_not_ready_streak.remove(&pool_id);
            true
        } else {
            false
        }
    }

    /// Record the current slot, for evaluating `m_pool_cooldown` entries.
    /// Callers should call this once per commit/low_latency batch, before
    /// running the slippage-aware search.
    pub fn set_current_slot(&mut self, slot: Slot) {
        self.current_slot = slot;
    }

    /// Exclude `pool_id` from the slippage-aware search for
    /// `cooldown_slots` slots from now (the caller decides the duration --
    /// this function is mechanism, not policy). Repeated calls extend
    /// (or shorten, if called with a smaller duration) the existing
    /// cooldown rather than stacking.
    pub fn mark_pool_cooldown(&mut self, pool_id: AccountId, cooldown_slots: u64) {
        self.m_pool_cooldown
            .insert(pool_id, self.current_slot.saturating_add(cooldown_slots));
    }

    /// Cap on how many times `mark_pool_not_ready_cooldown` doubles a
    /// pool's cooldown duration before holding steady -- at
    /// `POOL_COOLDOWN_SLOTS` (4500, ~30 min), 6 doublings caps out at
    /// 144,000 slots (~16 hours). A real, live-confirmed pool
    /// (`488251109`) failed `PoolNotReady` identically across 25+ process
    /// restarts spanning many hours -- a flat 30-minute cooldown just
    /// expires and gets re-applied unchanged forever for a pool that
    /// never recovers, so this escalates instead, while still bounded
    /// (not literally infinite) for a pool that's genuinely just been
    /// unusually slow.
    pub const POOL_NOT_READY_MAX_COOLDOWN_CYCLES: u32 = 6;

    /// Like [`Self::mark_pool_cooldown`], but doubles `base_cooldown_slots`
    /// every time the *same* pool crosses [`Self::note_pool_not_ready`]'s
    /// threshold again, capped at [`Self::POOL_NOT_READY_MAX_COOLDOWN_CYCLES`]
    /// doublings. Every caller that cools a pool down specifically because
    /// `note_pool_not_ready` returned `true` should call this instead of
    /// `mark_pool_cooldown` directly -- see this struct's
    /// `m_pool_not_ready_cooldown_cycles` field and this function's own
    /// doc comment above for why a flat duration isn't enough here.
    pub fn mark_pool_not_ready_cooldown(&mut self, pool_id: AccountId, base_cooldown_slots: u64) {
        let cycle = self.m_pool_not_ready_cooldown_cycles.entry(pool_id).or_insert(0);
        *cycle = (*cycle + 1).min(Self::POOL_NOT_READY_MAX_COOLDOWN_CYCLES);
        let escalated = base_cooldown_slots.saturating_mul(1u64 << (*cycle - 1));
        self.mark_pool_cooldown(pool_id, escalated);
    }

    /// Whether `pool_id` is currently excluded from the slippage-aware
    /// search by an active cooldown.
    fn is_cooled_down(&self, pool_id: AccountId) -> bool {
        self.m_pool_cooldown
            .get(&pool_id)
            .is_some_and(|&until| self.current_slot < until)
    }

    /// Seed a router whose node set is exactly `router::Router`'s known
    /// (liquidity-classified) mint universe -- Tier1 core + Tier2 cluster +
    /// Tier3 spoke tokens, from the (widest-path-priced) build-time
    /// ROUTER_POOLS snapshot. No edges yet -- `batch_router`'s
    /// `add_*_pool` calls populate those from live pool state every
    /// commit, and silently skip any pool touching a mint outside this
    /// universe (same "silently skips" convention `router::Router::update`
    /// already uses), so a token router::Router never saw real liquidity
    /// for can't enter the live trade graph either.
    pub fn from_router(router: &crate::trader::router::Router) -> Self {
        let mut tr = Self::new();
        for mint in router.known_mints() {
            tr.seed_node(mint);
        }
        tr
    }

    /// Register `token` as a node if not already known -- only ever called
    /// while seeding from router::Router (`from_router`). Live pool
    /// registration (`add_*_pool`) uses the lookup-only `node` below
    /// instead, since the node set must stay fixed to router::Router's
    /// classified universe after seeding.
    fn seed_node(&mut self, token: AccountId) -> usize {
        if let Some(&idx) = self.token_index.get(&token) {
            return idx;
        }
        let idx = self.tokens.len();
        self.tokens.push(token);
        self.token_index.insert(token, idx);
        self.edges.push(Vec::new());
        idx
    }

    /// Look up an already-seeded token's node index. Returns `None` for
    /// any mint outside router::Router's classified universe --
    /// `add_*_pool` callers silently skip the pool in that case.
    fn node(&self, token: AccountId) -> Option<usize> {
        self.token_index.get(&token).copied()
    }

    /// Total live edges across every node -- diagnostic only, for logging
    /// whether `batch_router` is actually populating anything this commit.
    pub fn edge_count(&self) -> usize {
        self.edges.iter().map(|e| e.len()).sum()
    }

    /// Outgoing edge count for one mint (0 if the mint isn't a node at
    /// all, i.e. router::Router never classified it) -- diagnostic only.
    pub fn out_degree(&self, token: AccountId) -> usize {
        self.node(token).map(|i| self.edges[i].len()).unwrap_or(0)
    }

    /// Whether `token` is a registered graph node -- diagnostic only, for
    /// per-dex modules to distinguish "no node for this mint" from "node
    /// exists but no live reserve data yet" when their batch_router isn't
    /// producing edges.
    pub fn has_node(&self, token: AccountId) -> bool {
        self.node(token).is_some()
    }

    /// Re-quote a single already-known edge (identified the same way a
    /// `Hop` identifies it: `input_mint` + `pool_id` + `output_mint`)
    /// against a possibly-different `amount_in`. Used by
    /// `planner::reverify_with_exact_quotes` to propagate a corrected
    /// upstream amount through the rest of a cycle's non-CLMM hops --
    /// `cp_quote` is already the *exact* formula for those (only CLMM
    /// pools have the tick-unaware-approximation gap that needs a real
    /// exact-quote correction in the first place). Returns `None` if the
    /// edge no longer exists (e.g. it went invalid between the original
    /// search and this re-check).
    pub fn requote_edge(
        &self,
        input_mint: AccountId,
        pool_id: AccountId,
        output_mint: AccountId,
        amount_in: u64,
    ) -> Option<u64> {
        let from = self.node(input_mint)?;
        let edge = self.edges[from]
            .iter()
            .find(|e| e.pool_id == pool_id && e.output_mint == output_mint)?;
        Some(cp_quote(amount_in, edge.reserve_in, edge.reserve_out, edge.fee_bps))
    }

    /// Re-quote `route`'s exact same sequence of hops (same pools, same
    /// order) at a different `amount_in`, via [`Self::requote_edge`] hop
    /// by hop -- **without** re-running `route_slippage_aware`, which
    /// would let a different `amount_in` pick a structurally different
    /// path. That matters because `widest_path`'s layered DP doesn't
    /// exclude revisited nodes (see its own doc comment); at some trade
    /// sizes the "best" path it finds can loop back through an
    /// already-visited token (observed live: a sell-direction quote for
    /// a real curated mint routed `mint -> USDC -> X -> USDC`, revisiting
    /// USDC mid-route). `factor_sizing::price_impact_bps` used to call
    /// `route_slippage_aware` independently for its probe and full
    /// quotes, so a probe picking the plain direct pool and a full quote
    /// picking that kind of revisiting path produced a "price impact"
    /// comparing two unrelated routes, not real same-path slippage --
    /// confirmed live via `route_diagnostics` output. This is the fix:
    /// find the route once (for whichever amount is being sized), then
    /// always requote the *other* amount along that identical path.
    ///
    /// `None` if any hop's edge no longer exists (e.g. removed between
    /// when `route` was found and this call) -- same "unpriceable, don't
    /// treat as zero-impact" contract as `requote_edge`.
    pub fn requote_along(&self, route: &Route, mut amount_in: u64) -> Option<u64> {
        for hop in &route.hops {
            amount_in = self.requote_edge(hop.input_mint, hop.pool_id, hop.output_mint, amount_in)?;
        }
        Some(amount_in)
    }

    /// One-off diagnostic for a live "no route found" failure -- reports
    /// whether `from_mint`/`to_mint` are indexed at all, and a *capped*
    /// sample of live (non-cooled-down) outgoing edges from `from_mint`'s
    /// node and incoming edges to `to_mint`'s node -- lets a caller tell
    /// apart "these tokens aren't both in the graph", "no direct pool, and
    /// no <=4-hop path either", and "a path exists but every hop is
    /// currently cooled down" without needing a debugger against the live
    /// bot process.
    ///
    /// **Must stay well under `stdio::MESSAGE_MAX_SIZE` (4KB)** -- a single
    /// `log_error!`/`log_warn!` call writes its whole formatted message
    /// into one fixed `STDIO_PACKET_BUFFER_MAX` (8KB) buffer in one shot
    /// (`StdioPacket::append`), with no internal chunking; a message
    /// bigger than that buffer panics the whole WASM guest instead of
    /// just failing to log (real, live-observed: an earlier, unbounded
    /// version of this method -- scanning every edge in the whole graph
    /// for "points at to_mint", unbounded for a heavily-connected token
    /// like SOL -- produced a 140KB+ string and crashed `testperpv1`
    /// mid-run with "range end index 143965 out of range for slice of
    /// length 8192"). `MAX_EDGES_SHOWN` hard-caps both scans so this can
    /// never again produce more than a small, fixed number of lines
    /// regardless of how connected either token is.
    pub fn route_diagnostics(&self, from_mint: AccountId, to_mint: AccountId, amount_in: u64, max_hops: usize) -> String {
        const MAX_EDGES_SHOWN: usize = 8;
        let Some(&src) = self.token_index.get(&from_mint) else {
            return format!("from_mint {from_mint} not indexed in router at all");
        };
        let Some(&dst) = self.token_index.get(&to_mint) else {
            return format!("to_mint {to_mint} not indexed in router at all");
        };
        let mut out = String::new();
        out.push_str(&format!("from_mint node={src} to_mint node={dst}\n"));

        // Replays route_slippage_aware's exact widest_path + reconstruction
        // steps. Reconstruction is now structurally guaranteed to reach
        // `src` whenever `best[max_hops][dst] != 0` (see `widest_path`'s
        // doc comment -- depth-tagged `pred` entries can't cycle), so this
        // is mainly useful to see the actual winning path/pools when
        // `route_slippage_aware` still returns "no route found" for some
        // other reason (e.g. every candidate quoting to 0 further down the
        // pipeline).
        let (best, pred) = self.widest_path(src, amount_in, max_hops);
        out.push_str(&format!(
            "widest_path best[max_hops][dst]={} (0 = no path found within {max_hops} hops)\n",
            best[max_hops][dst]
        ));
        if best[max_hops][dst] != 0 {
            let mut cur = dst;
            let mut k = max_hops;
            let mut trace = String::new();
            while cur != src {
                let Some((prev, ei, prev_k)) = pred[k][cur] else {
                    trace.push_str(&format!("cur={cur} k={k} has NO pred entry (unexpected)"));
                    break;
                };
                let edge = self.edges[prev].get(ei);
                trace.push_str(&format!(
                    "cur={cur} <- prev={prev} (edge idx {ei}: {}); ",
                    match edge {
                        Some(e) => format!(
                            "dex={:?} pool={} fee_bps={} reserve_in={} reserve_out={} cooled_down={}",
                            e.dex, e.pool_id, e.fee_bps, e.reserve_in, e.reserve_out, self.is_cooled_down(e.pool_id)
                        ),
                        None => "MISSING EDGE (index out of range)".to_string(),
                    }
                ));
                cur = prev;
                k = prev_k;
            }
            out.push_str(&format!("reconstruction trace (final cur={cur}): {trace}\n"));
        }

        // Targeted, bounded (single vec each, no printing) checks for the
        // two most decision-relevant facts a random 8-edge sample could
        // easily miss entirely: is there a *direct* pool between these
        // two tokens, and how many distinct 2-hop bridges exist (a node
        // reachable in one hop from `src` that also reaches `dst` in one
        // hop) -- if that count is 0, a >=3-hop path is the only
        // possibility even though both tokens individually have huge
        // degree.
        let direct: Vec<&RouterEdge> = self.edges[src].iter().filter(|e| e.to == dst).collect();
        out.push_str(&format!(
            "direct edges from_mint -> to_mint ({} total, showing up to {}), cp_quote({amount_in}, ...):\n",
            direct.len(),
            MAX_EDGES_SHOWN
        ));
        for e in direct.iter().take(MAX_EDGES_SHOWN) {
            let cooled = self.is_cooled_down(e.pool_id);
            let quote = cp_quote(amount_in, e.reserve_in, e.reserve_out, e.fee_bps);
            out.push_str(&format!(
                "  direct: dex={:?} pool={} cooled_down={} fee_bps={} reserve_in={} reserve_out={} quote_out={}\n",
                e.dex, e.pool_id, cooled, e.fee_bps, e.reserve_in, e.reserve_out, quote
            ));
        }
        let reachable_from_src: std::collections::HashSet<usize> =
            self.edges[src].iter().filter(|e| !self.is_cooled_down(e.pool_id)).map(|e| e.to).collect();
        let bridge_count = self
            .edges
            .iter()
            .enumerate()
            .filter(|(u, adj)| {
                reachable_from_src.contains(u) && adj.iter().any(|e| e.to == dst && !self.is_cooled_down(e.pool_id))
            })
            .count();
        out.push_str(&format!(
            "live (non-cooled-down) 2-hop bridge nodes (reachable from from_mint AND reach to_mint): {bridge_count}\n"
        ));

        out.push_str(&format!(
            "outgoing edges from from_mint ({} total, showing up to {}):\n",
            self.edges[src].len(),
            MAX_EDGES_SHOWN
        ));
        for e in self.edges[src].iter().take(MAX_EDGES_SHOWN) {
            let cooled = self.is_cooled_down(e.pool_id);
            out.push_str(&format!(
                "  -> node={} dex={:?} pool={} output_mint={} cooled_down={} reserve_in={} reserve_out={}\n",
                e.to, e.dex, e.pool_id, e.output_mint, cooled, e.reserve_in, e.reserve_out
            ));
        }
        let incoming: Vec<(usize, &RouterEdge)> = self
            .edges
            .iter()
            .enumerate()
            .flat_map(|(u, adj)| adj.iter().map(move |e| (u, e)))
            .filter(|(_, e)| e.to == dst)
            .collect();
        out.push_str(&format!(
            "incoming edges to to_mint ({} total, showing up to {}):\n",
            incoming.len(),
            MAX_EDGES_SHOWN
        ));
        for (u, e) in incoming.iter().take(MAX_EDGES_SHOWN) {
            let cooled = self.is_cooled_down(e.pool_id);
            out.push_str(&format!(
                "  node={u} -> dex={:?} pool={} input_mint={} cooled_down={} reserve_in={} reserve_out={}\n",
                e.dex, e.pool_id, e.input_mint, cooled, e.reserve_in, e.reserve_out
            ));
        }
        out
    }

    /// Live edge counts grouped by DEX -- diagnostic only, to see whether a
    /// specific DEX (e.g. Sanctum) is actually contributing edges this
    /// commit rather than just having its pools/LSTs loaded.
    pub fn edge_counts_by_dex(&self) -> Vec<(DexType, usize)> {
        let mut counts: HashMap<DexType, usize> = HashMap::new();
        for adj in &self.edges {
            for e in adj {
                *counts.entry(e.dex).or_insert(0) += 1;
            }
        }
        let mut v: Vec<(DexType, usize)> = counts.into_iter().collect();
        v.sort_by_key(|(d, _)| format!("{d:?}"));
        v
    }

    /// Insert/replace the edge from node `from` identified by
    /// `(pool_id, output_mint)` -- **not** `pool_id` alone, since Sanctum
    /// registers every LST pair under one shared `pool_id`
    /// (`pool_state_id`); keying on `pool_id` alone would wipe every other
    /// pair sharing that id when only one pair's price actually changed.
    /// `input_mint` isn't part of the key: within `edges[from]` it's
    /// always `tokens[from]` by construction, so it can't disambiguate
    /// anything `output_mint` doesn't already.
    ///
    /// Callers must upsert (not blind-push) so repeated incremental calls
    /// for the same pool -- driven by live per-account/per-token updates,
    /// not a periodic `clear()`-then-rebuild -- never accumulate
    /// duplicate edges.
    fn upsert_edge(&mut self, from: usize, edge: RouterEdge) {
        self.remove_edge(from, edge.pool_id, edge.output_mint);
        self.edges[from].push(edge);
    }

    /// Remove the edge from node `from` identified by `(pool_id,
    /// output_mint)`, if present. Callers must invoke this on a pool
    /// invalidity gate (rather than a blind early return) so a pool that
    /// was live and then goes invalid -- genuinely drained, or a bad
    /// transient read -- doesn't leave a stale, wrong-priced edge in the
    /// graph forever.
    fn remove_edge(&mut self, from: usize, pool_id: AccountId, output_mint: AccountId) {
        self.edges[from].retain(|e| !(e.pool_id == pool_id && e.output_mint == output_mint));
    }

    /// Register one Orca Whirlpool as two directed edges (A→B, B→A).
    ///
    /// Uses `pool.virtual_reserves()` (derived from `liquidity`/
    /// `sqrt_price_x64`, the same account snapshot `spot_price()` comes
    /// from) for `cp_quote` sizing -- **not** `pool.reserve_a`/`reserve_b`
    /// (the real vault balances). See `virtual_reserves()`'s doc comment:
    /// a Whirlpool's vaults hold every LP's liquidity across the entire
    /// price range, not just what's active at the current tick, so a pool
    /// with a lot of out-of-range liquidity can have a vault-balance ratio
    /// wildly inconsistent with its real tradeable liquidity even though
    /// `spot_price()` is completely correct -- confirmed live (100 SOL
    /// "quoting" as both ~2.99e15 and ~1.58e-3 raw-unit outputs through
    /// different pools, both collapsing back to a sane number once
    /// switched to virtual reserves, which are guaranteed self-consistent
    /// with `spot_price()` by construction). This also means a pool no
    /// longer needs to wait on vault-balance delivery (on_token) before
    /// it's quotable here at all -- `liquidity`/`sqrt_price_x64` alone,
    /// straight from `on_account`, are now sufficient.
    pub fn add_orca_pool(&mut self, pool_id: AccountId, pool: &OrcaWhirlpool, dex: DexType) {
        let (Some(a), Some(b)) = (self.node(pool.token_mint_a), self.node(pool.token_mint_b))
        else {
            return; // mint outside router's classified universe -- nothing to remove either
        };

        let spot = pool.spot_price();
        let (virtual_a, virtual_b) = pool.virtual_reserves();
        if spot <= 0.0 || pool.liquidity == 0 || virtual_a == 0 || virtual_b == 0 {
            self.remove_edge(a, pool_id, pool.token_mint_b);
            self.remove_edge(b, pool_id, pool.token_mint_a);
            return;
        }
        // fee_rate is in hundredths of a basis point (e.g. 3000 = 0.30%).
        let fee_frac = pool.fee_rate as f64 / 1_000_000.0;
        let fee_bps = pool.fee_bps();

        // A → B
        let rate_ab = spot * (1.0 - fee_frac);
        self.upsert_edge(
            a,
            RouterEdge {
                to: b,
                neg_log_rate: -rate_ab.ln(),
                pool_id,
                input_mint: pool.token_mint_a,
                output_mint: pool.token_mint_b,
                reserve_in: virtual_a,
                reserve_out: virtual_b,
                fee_bps,
                dex,
            },
        );

        // B → A
        let rate_ba = (1.0 / spot) * (1.0 - fee_frac);
        self.upsert_edge(
            b,
            RouterEdge {
                to: a,
                neg_log_rate: -rate_ba.ln(),
                pool_id,
                input_mint: pool.token_mint_b,
                output_mint: pool.token_mint_a,
                reserve_in: virtual_b,
                reserve_out: virtual_a,
                fee_bps,
                dex,
            },
        );
    }

    /// Register one Raydium CLMM pool as two directed edges (0→1, 1→0).
    ///
    /// `fee_rate_pips`: parts-per-million fee (e.g. 2500 = 0.25%). Comes from
    /// `RaydiumClmmPoolSetup::fee_rate_pips` since it lives in AmmConfig, not pool state.
    pub fn add_raydium_clmm_pool(
        &mut self,
        pool_id: AccountId,
        pool: &RaydiumClmmPool,
        fee_rate_pips: u32,
        dex: DexType,
    ) {
        let (Some(a), Some(b)) = (self.node(pool.token_mint_0), self.node(pool.token_mint_1))
        else {
            return;
        };

        let spot = pool.spot_price();
        // Same reasoning as add_orca_pool's gate: `liquidity` is native
        // on-chain pool state, but reserve_0/reserve_1 come from separate
        // live vault-balance tracking that can still be zero for a pool
        // whose account has already been read.
        if spot <= 0.0 || pool.liquidity == 0 || pool.reserve_0 == 0 || pool.reserve_1 == 0 {
            self.remove_edge(a, pool_id, pool.token_mint_1);
            self.remove_edge(b, pool_id, pool.token_mint_0);
            return;
        }
        let fee_frac = fee_rate_pips as f64 / 1_000_000.0;
        let fee_bps = (fee_rate_pips / 100) as u16;

        // 0 → 1
        let rate_01 = spot * (1.0 - fee_frac);
        self.upsert_edge(
            a,
            RouterEdge {
                to: b,
                neg_log_rate: -rate_01.ln(),
                pool_id,
                input_mint: pool.token_mint_0,
                output_mint: pool.token_mint_1,
                reserve_in: pool.reserve_0,
                reserve_out: pool.reserve_1,
                fee_bps,
                dex,
            },
        );

        // 1 → 0
        let rate_10 = (1.0 / spot) * (1.0 - fee_frac);
        self.upsert_edge(
            b,
            RouterEdge {
                to: a,
                neg_log_rate: -rate_10.ln(),
                pool_id,
                input_mint: pool.token_mint_1,
                output_mint: pool.token_mint_0,
                reserve_in: pool.reserve_1,
                reserve_out: pool.reserve_0,
                fee_bps,
                dex,
            },
        );
    }

    /// Register a single directed pair as two edges (src→dst and dst→src).
    ///
    /// Used by DEX modules (e.g. Sanctum) that manage their own pool logic but
    /// need to expose pairs to the router for arbitrage detection.
    ///
    /// `price_b_per_a`: dst raw units per src raw unit (before fees).
    /// `fee_frac`: fractional fee, e.g. `0.003` for 0.3%.
    pub fn add_generic_pair(
        &mut self,
        pool_id: AccountId,
        token_a: AccountId,
        token_b: AccountId,
        price_b_per_a: f64,
        fee_frac: f64,
        reserve_a: u64,
        reserve_b: u64,
        dex: DexType,
    ) {
        let (Some(a), Some(b)) = (self.node(token_a), self.node(token_b)) else {
            return;
        };

        if price_b_per_a <= 0.0 || !(0.0..1.0).contains(&fee_frac) {
            self.remove_edge(a, pool_id, token_b);
            self.remove_edge(b, pool_id, token_a);
            return;
        }
        let fee_bps = (fee_frac * 10_000.0) as u16;

        let rate_ab = price_b_per_a * (1.0 - fee_frac);
        self.upsert_edge(
            a,
            RouterEdge {
                to: b,
                neg_log_rate: -rate_ab.ln(),
                pool_id,
                input_mint: token_a,
                output_mint: token_b,
                reserve_in: reserve_a,
                reserve_out: reserve_b,
                fee_bps,
                dex,
            },
        );

        let rate_ba = (1.0 / price_b_per_a) * (1.0 - fee_frac);
        self.upsert_edge(
            b,
            RouterEdge {
                to: a,
                neg_log_rate: -rate_ba.ln(),
                pool_id,
                input_mint: token_b,
                output_mint: token_a,
                reserve_in: reserve_b,
                reserve_out: reserve_a,
                fee_bps,
                dex,
            },
        );
    }

    /// Register a single one-directional edge (`input_mint` → `output_mint`
    /// only, no reverse) -- for venues where the reverse trade genuinely
    /// isn't possible through this same mechanism, e.g. an LST's instant
    /// liquid-unstake redemption (LST → SOL only; there's no "mint LST via
    /// liquid-unstake" in reverse). `add_generic_pair` always adds both
    /// directions, which would be wrong here.
    ///
    /// `price_out_per_in`: output raw units per input raw unit (before fees).
    /// `fee_frac`: fractional fee, e.g. `0.001` for 0.1%.
    pub fn add_directed_edge(
        &mut self,
        pool_id: AccountId,
        input_mint: AccountId,
        output_mint: AccountId,
        price_out_per_in: f64,
        fee_frac: f64,
        reserve_in: u64,
        reserve_out: u64,
        dex: DexType,
    ) {
        let (Some(a), Some(b)) = (self.node(input_mint), self.node(output_mint)) else {
            return;
        };

        if price_out_per_in <= 0.0 || !(0.0..1.0).contains(&fee_frac) {
            self.remove_edge(a, pool_id, output_mint);
            return;
        }
        let fee_bps = (fee_frac * 10_000.0) as u16;

        let rate = price_out_per_in * (1.0 - fee_frac);
        self.upsert_edge(
            a,
            RouterEdge {
                to: b,
                neg_log_rate: -rate.ln(),
                pool_id,
                input_mint,
                output_mint,
                reserve_in,
                reserve_out,
                fee_bps,
                dex,
            },
        );
    }

    /// Register one Raydium AMM v4 (constant-product) pool as two directed edges.
    pub fn add_raydium_amm_pool(&mut self, pool_id: AccountId, pool: &RaydiumAmmPool, dex: DexType) {
        let (Some(a), Some(b)) = (self.node(pool.coin_mint), self.node(pool.pc_mint)) else {
            return;
        };

        if pool.reserve_coin == 0 || pool.reserve_pc == 0 {
            self.remove_edge(a, pool_id, pool.pc_mint);
            self.remove_edge(b, pool_id, pool.coin_mint);
            return;
        }
        let spot = pool.reserve_pc as f64 / pool.reserve_coin as f64;
        if spot <= 0.0 {
            self.remove_edge(a, pool_id, pool.pc_mint);
            self.remove_edge(b, pool_id, pool.coin_mint);
            return;
        }
        let fee_frac = if pool.swap_fee_denominator == 0 {
            0.0
        } else {
            pool.swap_fee_numerator as f64 / pool.swap_fee_denominator as f64
        };
        let fee_bps = pool.fee_bps();

        let rate_ab = spot * (1.0 - fee_frac);
        self.upsert_edge(
            a,
            RouterEdge {
                to: b,
                neg_log_rate: -rate_ab.ln(),
                pool_id,
                input_mint: pool.coin_mint,
                output_mint: pool.pc_mint,
                reserve_in: pool.reserve_coin,
                reserve_out: pool.reserve_pc,
                fee_bps,
                dex,
            },
        );

        let rate_ba = (1.0 / spot) * (1.0 - fee_frac);
        self.upsert_edge(
            b,
            RouterEdge {
                to: a,
                neg_log_rate: -rate_ba.ln(),
                pool_id,
                input_mint: pool.pc_mint,
                output_mint: pool.coin_mint,
                reserve_in: pool.reserve_pc,
                reserve_out: pool.reserve_coin,
                fee_bps,
                dex,
            },
        );
    }

    /// Reset all edges without dropping heap allocations. The node set
    /// itself (router::Router's classified mint universe, from
    /// `from_router`) is deliberately left intact -- it's not live data,
    /// so there's nothing to refresh there each commit, and re-deriving it
    /// would just mean re-seeding the exact same tokens every time.
    pub fn clear(&mut self) {
        for e in &mut self.edges {
            e.clear();
        }
    }

    /// Find the best route from `from_mint` to `to_mint` for `amount_in` tokens.
    ///
    /// Uses Bellman-Ford (capped at `max_hops` relaxation rounds) on
    /// `−ln(spot × (1 − fee))` edge weights. The minimum-cost path in
    /// log-space corresponds to the maximum cumulative exchange rate.
    ///
    /// Once the path is found, output amounts are estimated using a
    /// constant-product AMM quote at each hop. For CLMM pools (Orca
    /// Whirlpools) this is an approximation; use the full quote via
    /// `OrcaState::swap` or `swap_quote_by_input_token` for exact amounts.
    ///
    /// Returns `None` if no path exists within `max_hops` hops.
    pub fn route(
        &self,
        from_mint: AccountId,
        to_mint: AccountId,
        amount_in: u64,
        max_hops: usize,
    ) -> Option<Route> {
        if max_hops == 0 || amount_in == 0 {
            return None;
        }
        let n = self.tokens.len();
        let &src = self.token_index.get(&from_mint)?;
        let &dst = self.token_index.get(&to_mint)?;
        if src == dst {
            return None;
        }

        // dist[u] = minimum accumulated −ln(rate) from src to u.
        let mut dist = vec![f64::INFINITY; n];
        // pred[u] = (predecessor node, edge index in edges[predecessor]).
        let mut pred: Vec<Option<(usize, usize)>> = vec![None; n];
        dist[src] = 0.0;

        // Bellman-Ford capped at max_hops rounds.
        // After k rounds, dist[u] reflects the best path of ≤ k edges.
        for _ in 0..max_hops {
            let mut changed = false;
            for u in 0..n {
                if dist[u].is_infinite() {
                    continue;
                }
                for (ei, edge) in self.edges[u].iter().enumerate() {
                    let v = edge.to;
                    let new_dist = dist[u] + edge.neg_log_rate;
                    if new_dist < dist[v] {
                        dist[v] = new_dist;
                        pred[v] = Some((u, ei));
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }

        if dist[dst].is_infinite() {
            return None;
        }

        // Reconstruct path by walking predecessors from dst back to src.
        // Guarded by max_hops to prevent infinite loops on malformed graphs.
        let mut path: Vec<(usize, usize)> = Vec::with_capacity(max_hops);
        let mut cur = dst;
        for _ in 0..max_hops {
            if cur == src {
                break;
            }
            let (prev, ei) = pred[cur]?;
            path.push((prev, ei));
            cur = prev;
        }
        if cur != src {
            return None; // path too long or broken
        }
        path.reverse();

        // Forward pass: cascade amounts through each hop.
        let mut hops = Vec::with_capacity(path.len());
        let mut amt = amount_in;
        for (u, ei) in &path {
            let edge = &self.edges[*u][*ei];
            let out = cp_quote(amt, edge.reserve_in, edge.reserve_out, edge.fee_bps);
            hops.push(Hop {
                pool_id: edge.pool_id,
                input_mint: edge.input_mint,
                output_mint: edge.output_mint,
                amount_in: amt,
                amount_out: out,
                dex: edge.dex,
            });
            amt = out;
            if amt == 0 {
                // Reserves too thin for this amount; no usable route.
                return None;
            }
        }

        Some(Route { hops })
    }

    /// Diagnostic-only twin of `route` -- instead of collapsing every
    /// failure mode to `None`, says WHICH stage failed (no node, no path
    /// within max_hops, broken path reconstruction, or a real path whose
    /// cp-AMM cascade zeroed out partway through). Call only when `route`
    /// returns `None` and you need to know why; not meant for the hot path.
    pub fn route_diagnose(
        &self,
        from_mint: AccountId,
        to_mint: AccountId,
        amount_in: u64,
        max_hops: usize,
    ) -> String {
        let Some(&src) = self.token_index.get(&from_mint) else {
            return "from_mint is not a node (router::Router never classified it)".to_string();
        };
        let Some(&dst) = self.token_index.get(&to_mint) else {
            return "to_mint is not a node (router::Router never classified it)".to_string();
        };
        if src == dst {
            return "from_mint == to_mint".to_string();
        }
        let n = self.tokens.len();
        let mut dist = vec![f64::INFINITY; n];
        let mut pred: Vec<Option<(usize, usize)>> = vec![None; n];
        dist[src] = 0.0;
        for _ in 0..max_hops {
            let mut changed = false;
            for u in 0..n {
                if dist[u].is_infinite() {
                    continue;
                }
                for (ei, edge) in self.edges[u].iter().enumerate() {
                    let v = edge.to;
                    let new_dist = dist[u] + edge.neg_log_rate;
                    if new_dist < dist[v] {
                        dist[v] = new_dist;
                        pred[v] = Some((u, ei));
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        if dist[dst].is_infinite() {
            return format!(
                "bellman-ford found no path within {max_hops} hops \
                 (src_out_degree={}, dst_out_degree={})",
                self.edges[src].len(),
                self.edges[dst].len(),
            );
        }
        let mut path: Vec<(usize, usize)> = Vec::with_capacity(max_hops);
        let mut cur = dst;
        for _ in 0..max_hops {
            if cur == src {
                break;
            }
            let Some((prev, ei)) = pred[cur] else {
                return "path reconstruction broke: missing predecessor".to_string();
            };
            path.push((prev, ei));
            cur = prev;
        }
        if cur != src {
            return format!("path reconstruction exceeded max_hops={max_hops}");
        }
        path.reverse();
        let mut amt = amount_in;
        for (i, (u, ei)) in path.iter().enumerate() {
            let edge = &self.edges[*u][*ei];
            let out = cp_quote(amt, edge.reserve_in, edge.reserve_out, edge.fee_bps);
            if out == 0 {
                return format!(
                    "path found ({} hops) but hop {i} zeroed the amount \
                     (amount_in={amt}, reserve_in={}, reserve_out={}, fee_bps={})",
                    path.len(),
                    edge.reserve_in,
                    edge.reserve_out,
                    edge.fee_bps,
                );
            }
            amt = out;
        }
        format!(
            "inconsistent: route_diagnose found a working {}-hop path (final amount {amt}) \
             but route() returned None",
            path.len()
        )
    }

    /// Detect and return one profitable arbitrage cycle, if any exists.
    ///
    /// Runs Bellman-Ford with all nodes initialised to distance 0 (equivalent
    /// to a virtual source connected to every node at zero cost). Any edge
    /// still relaxable after N rounds lies on or downstream of a negative cycle
    /// in log-space — i.e. a sequence of swaps whose cumulative rate > 1.
    ///
    /// The cp-AMM cascade is then applied to `amount_in` raw units of the cycle's
    /// starting token to verify the cycle is profitable after fees and thin liquidity.
    /// Returns `None` if no profitable cycle is found.
    pub fn find_arbitrage(&self, amount_in: u64) -> Option<ArbitrageCycle> {
        let n = self.tokens.len();
        if n < 2 || amount_in == 0 {
            return None;
        }

        let mut dist = vec![0.0f64; n];
        let mut pred: Vec<Option<(usize, usize)>> = vec![None; n];
        let mut last_relaxed: Option<usize> = None;

        for round in 0..n {
            for u in 0..n {
                for (ei, edge) in self.edges[u].iter().enumerate() {
                    let v = edge.to;
                    let new_dist = dist[u] + edge.neg_log_rate;
                    if new_dist < dist[v] {
                        dist[v] = new_dist;
                        pred[v] = Some((u, ei));
                        if round == n - 1 {
                            last_relaxed = Some(v);
                        }
                    }
                }
            }
        }

        // No node relaxed in the Nth round → no negative cycle.
        let mut cur = last_relaxed?;

        // Walk back n steps through predecessors to land on a node that is
        // guaranteed to be part of the cycle (not just reachable from it).
        for _ in 0..n {
            let (prev, _) = pred[cur]?;
            cur = prev;
        }

        // Reconstruct the cycle by following predecessors until we return.
        let cycle_start = cur;
        let mut path: Vec<(usize, usize)> = Vec::new();
        loop {
            let (prev, ei) = pred[cur]?;
            path.push((prev, ei));
            cur = prev;
            if cur == cycle_start {
                break;
            }
            if path.len() > n {
                return None; // safety guard
            }
        }
        path.reverse(); // predecessors are backwards; reverse to get forward order

        // Cascade cp_quote through the cycle with the requested input amount.
        let mut hops = Vec::with_capacity(path.len());
        let mut amt = amount_in;
        for (u, ei) in &path {
            let edge = &self.edges[*u][*ei];
            let out = cp_quote(amt, edge.reserve_in, edge.reserve_out, edge.fee_bps);
            hops.push(Hop {
                pool_id: edge.pool_id,
                input_mint: edge.input_mint,
                output_mint: edge.output_mint,
                amount_in: amt,
                amount_out: out,
                dex: edge.dex,
            });
            amt = out;
            if amt == 0 {
                return None; // reserves too thin
            }
        }

        // Verify cycle is actually profitable after fees and real liquidity.
        if amt <= amount_in {
            return None;
        }

        Some(ArbitrageCycle { hops })
    }

    /// Shared relaxation core for the slippage-aware `route`/`find_arbitrage`
    /// variants below. Unlike `-ln(rate)` log-space distances (amount-
    /// independent, but blind to slippage since summing weights never
    /// accounts for how much is actually flowing through an edge), this
    /// tracks the real `cp_quote`-composed amount reachable at each node
    /// for a *fixed* starting `amount_in` at `src`. It's a "widest path" /
    /// bottleneck-shortest-path DP: `cp_quote` is monotonic in its own
    /// `amount_in` (more in never yields less out), which preserves the
    /// optimal-substructure property Bellman-Ford-style relaxation needs --
    /// just "maximize amount" in place of "minimize distance sum."
    ///
    /// Capped at `max_hops` rounds, not `n` -- unlike `find_arbitrage`'s
    /// negative-cycle search (which doesn't know a cycle's length in
    /// advance, so needs up to `n` rounds to guarantee catching one
    /// anywhere), callers here already bound path length, so `max_hops`
    /// rounds is enough to consider every path/cycle up to that length.
    ///
    /// `best[v] == 0` doubles as "not usefully reachable" (matches
    /// `route`/`find_arbitrage`'s existing convention of treating a
    /// `cp_quote` result of exactly 0 as "reserves too thin, no usable
    /// route" rather than a real quote).
    ///
    /// Returns per-hop-depth `best`/`pred` tables: `best[k][v]` is the
    /// highest amount reachable at node `v` using *up to* `k` hops from
    /// `src` (monotonically non-decreasing in `k`, since `best[k]` starts
    /// as a carried-forward copy of `best[k-1]` before any hop-`k` edge is
    /// tried), and `pred[k][v]` is `Some((prev, edge_index,
    /// prev_hop_depth))` -- `prev_hop_depth` is always `< k`, whether the
    /// entry was just set by a real hop-`k` relaxation (`prev_hop_depth =
    /// k - 1`) or simply carried forward unchanged from `pred[k-1][v]`
    /// (whatever smaller depth *that* was originally set at).
    ///
    /// This depth-tagging is required for correctness, not just style: an
    /// earlier version kept a single evolving `best`/`pred` pair (snapshotted
    /// once per round to stop same-round chaining, but still overwritten
    /// in place across rounds). That's insufficient whenever a node's value
    /// improves at *more than one* round via *different* predecessors --
    /// which a genuinely compounding positive-weight cycle does by
    /// construction (each additional round-trip round improves it further).
    /// Once `pred[v]` is overwritten by hop k+2's update, hop k's earlier
    /// update -- and whatever it pointed at -- is gone; the backward walk
    /// then follows a `pred` chain stitched together from *different*
    /// rounds' snapshots, which can legitimately cycle back on itself
    /// without ever reaching `src`, independent of whether each individual
    /// round's relaxation was itself correct. Real, live-confirmed
    /// consequence (two *separate* mispriced pairs, on two different
    /// restarts, both reproduced this exact failure mode): `best[dst]`
    /// nonzero and healthy, direct `src`->`dst` edges present with sane
    /// `cp_quote` outputs, yet reconstruction bounced between two nodes
    /// for all `max_hops` allotted steps and never reached `src` --
    /// `route_slippage_aware` returned "no route found" regardless.
    ///
    /// Tagging every `pred` entry with the exact depth it was set at fixes
    /// this structurally: a caller walking backward always jumps to a
    /// *strictly smaller* depth, so the walk is bounded by `max_hops` and
    /// provably cannot cycle, no matter how many times a node's value was
    /// independently improved across different rounds.
    fn widest_path(
        &self,
        src: usize,
        amount_in: u64,
        max_hops: usize,
    ) -> (Vec<Vec<u64>>, Vec<Vec<Option<(usize, usize, usize)>>>) {
        let n = self.tokens.len();
        let mut best: Vec<Vec<u64>> = vec![vec![0u64; n]; max_hops + 1];
        let mut pred: Vec<Vec<Option<(usize, usize, usize)>>> = vec![vec![None; n]; max_hops + 1];
        best[0][src] = amount_in;
        for k in 1..=max_hops {
            best[k] = best[k - 1].clone();
            pred[k] = pred[k - 1].clone();
            for u in 0..n {
                if best[k - 1][u] == 0 {
                    continue;
                }
                for (ei, edge) in self.edges[u].iter().enumerate() {
                    if self.is_cooled_down(edge.pool_id) {
                        continue;
                    }
                    // Real, live-confirmed bug this closes: comparing raw
                    // `cp_quote` output across candidate edges that end at
                    // *different* tokens (different decimals/real value
                    // per raw unit) lets a pool that's nearly drained
                    // relative to this hop's own input size look like the
                    // "best" edge purely as a constant-product-formula
                    // artifact -- not because it's actually deep or
                    // cheap. `path_revisits_a_node`'s own doc comment
                    // covers one shape of the resulting damage (a path
                    // looping back through an already-visited token); this
                    // is the more general root cause, confirmed live in a
                    // *non*-revisiting case too: a real 3-hop route for a
                    // normally-deep LST (no token touched twice) quoted an
                    // ~88x-inflated output because hop 1 alone traded a
                    // large fraction of its pool's own `reserve_in`, and
                    // the binary search in `factor_sizing::max_safe_
                    // notional` never discovered the sane, deep direct
                    // pool because this same inflated-looking edge kept
                    // "winning" at every size it probed. Capping how much
                    // of a pool's *own* reserve a single hop may consume
                    // is dimensionless (no USD price data needed, unlike a
                    // real minimum-liquidity-in-dollars floor) and fixes
                    // this at the source for every caller (`route_
                    // slippage_aware`, `find_arbitrage_slippage_aware`)
                    // uniformly, not just the two shapes observed so far.
                    if (best[k - 1][u] as u128) * 10_000 > (edge.reserve_in as u128) * MAX_HOP_POOL_UTILIZATION_BPS as u128 {
                        continue;
                    }
                    let v = edge.to;
                    let candidate = cp_quote(best[k - 1][u], edge.reserve_in, edge.reserve_out, edge.fee_bps);
                    if candidate > best[k][v] {
                        best[k][v] = candidate;
                        pred[k][v] = Some((u, ei, k - 1));
                    }
                }
            }
        }
        (best, pred)
    }

    /// Walks `pred` from `dst` back to `src` at hop-depth `k`, returning
    /// the `(node, edge_index)` chain in forward order. `None` if `pred`
    /// has no path recorded reaching `dst` within exactly `k` hops --
    /// callers should check `best[k][dst] != 0` first (see
    /// [`Self::widest_path`]'s doc comment for why the walk is bounded
    /// and provably cannot cycle).
    fn reconstruct_path(
        &self,
        pred: &[Vec<Option<(usize, usize, usize)>>],
        src: usize,
        dst: usize,
        k: usize,
    ) -> Option<Vec<(usize, usize)>> {
        let mut path = Vec::with_capacity(k);
        let mut cur = dst;
        let mut depth = k;
        while cur != src {
            let (prev, ei, prev_k) = pred[depth][cur]?;
            path.push((prev, ei));
            cur = prev;
            depth = prev_k;
        }
        path.reverse();
        Some(path)
    }

    /// Cycle-closing twin of [`Self::reconstruct_path`] for
    /// [`Self::find_arbitrage_slippage_aware`]: walks `pred` from `src`
    /// back to `src` at hop-depth `k` (at least one real hop, since a
    /// cycle can't be zero-length), returning the `(node, edge_index)`
    /// chain in forward order. `None` if `pred` has no such cycle
    /// recorded at exactly `k` hops -- callers should check
    /// `best[k][src] > amount_in` first.
    fn reconstruct_cycle(
        &self,
        pred: &[Vec<Option<(usize, usize, usize)>>],
        src: usize,
        k: usize,
    ) -> Option<Vec<(usize, usize)>> {
        let mut path = Vec::with_capacity(k);
        let mut cur = src;
        let mut depth = k;
        loop {
            let (prev, ei, prev_k) = pred[depth][cur]?;
            path.push((prev, ei));
            cur = prev;
            depth = prev_k;
            if cur == src {
                break;
            }
        }
        path.reverse();
        Some(path)
    }

    /// Slippage-aware twin of [`route`](Self::route): instead of picking
    /// the path with the best cumulative *spot* rate and only checking
    /// real slippage afterward, this compares actual `cp_quote`-composed
    /// amounts *during* the search, so it can find a path through deeper
    /// (but spot-price-worse) liquidity that `route` would never even
    /// consider if a shallower, better-spot-priced path exists in
    /// parallel. Costs one `cp_quote` call per edge per relaxation round
    /// (vs. a cheap running sum for `route`), and -- unlike `route` --
    /// its own path *selection* depends on `amount_in`, not just the
    /// final output estimate.
    pub fn route_slippage_aware(
        &self,
        from_mint: AccountId,
        to_mint: AccountId,
        amount_in: u64,
        max_hops: usize,
    ) -> Option<Route> {
        if max_hops == 0 || amount_in == 0 {
            return None;
        }
        let &src = self.token_index.get(&from_mint)?;
        let &dst = self.token_index.get(&to_mint)?;
        if src == dst {
            return None;
        }
        let (best, pred) = self.widest_path(src, amount_in, max_hops);
        if best[max_hops][dst] == 0 {
            return None;
        }

        // The widest-path DP has no "already used this pool" memory, so
        // its single best predecessor chain at `max_hops` can legitimately
        // revisit the same pool at two different hops (live-confirmed
        // this session: a real 4-hop reconstruction bounced through the
        // same pool at hops 2 and 4 because bouncing was locally
        // "improving" by the DP's own per-hop accounting, even though
        // it's unsafe to execute atomically -- see `hops_reuse_a_pool`'s
        // doc comment). Same fallback for the related but distinct
        // degenerate case `path_revisits_a_node` catches: three *different*
        // pools can still compose a route that passes through the same
        // token twice (live-confirmed: a real "best" path for
        // `mint -> USDC` was actually `mint -> USDC -> X -> USDC`,
        // quoting a wildly inflated output via a near-drained pool --
        // see that function's own doc comment). Rather than rejecting
        // outright the moment either happens (which reliably re-picks
        // the exact same unsafe route every retry when reserves haven't
        // moved enough to change the DP's answer, live-confirmed this
        // session to stall a real close leg indefinitely), fall back
        // through shorter hop counts -- `best[k]` is non-decreasing in
        // `k` (each round starts as a copy of the previous), so a
        // smaller `k` is a strictly shorter chain with less room to
        // accidentally repeat a pool or a node, at the cost of a
        // possibly-smaller output amount.
        let mut path: Option<Vec<(usize, usize)>> = None;
        for k in (1..=max_hops).rev() {
            if best[k][dst] == 0 {
                continue;
            }
            if let Some(candidate) = self.reconstruct_path(&pred, src, dst, k) {
                if !path_reuses_a_pool(&candidate, &self.edges) && !path_revisits_a_node(&candidate, src, &self.edges) {
                    path = Some(candidate);
                    break;
                }
            }
        }
        let path = path?;

        let mut hops = Vec::with_capacity(path.len());
        let mut amt = amount_in;
        for (u, ei) in &path {
            let edge = &self.edges[*u][*ei];
            let out = cp_quote(amt, edge.reserve_in, edge.reserve_out, edge.fee_bps);
            hops.push(Hop {
                pool_id: edge.pool_id,
                input_mint: edge.input_mint,
                output_mint: edge.output_mint,
                amount_in: amt,
                amount_out: out,
                dex: edge.dex,
            });
            amt = out;
            if amt == 0 {
                return None;
            }
        }
        Some(Route { hops })
    }

    /// Slippage-aware twin of [`find_arbitrage`](Self::find_arbitrage):
    /// searches for a profitable cycle starting and ending at `start_mint`
    /// using real `cp_quote`-composed amounts throughout the search
    /// (`widest_path`), rather than picking the single best-by-spot-rate
    /// cycle and only checking slippage as a final pass/fail gate.
    ///
    /// Needs a caller-supplied `start_mint` -- unlike `find_arbitrage`,
    /// which is amount-independent and so can search from every node
    /// simultaneously (log-space distances all start at the same
    /// "distance 0"), a real `cp_quote`-composed amount is only meaningful
    /// once you fix which token `amount_in` denominates. Callers that
    /// don't already know a specific token to check (e.g. `find_arbitrage`
    /// was previously used to cheaply discover "some cycle exists
    /// somewhere") should call this once per token they actually hold a
    /// balance in, not attempt every node in the graph.
    pub fn find_arbitrage_slippage_aware(
        &self,
        start_mint: AccountId,
        amount_in: u64,
        max_hops: usize,
    ) -> Option<ArbitrageCycle> {
        if self.tokens.len() < 2 || amount_in == 0 || max_hops == 0 {
            return None;
        }
        let &src = self.token_index.get(&start_mint)?;
        let (best, pred) = self.widest_path(src, amount_in, max_hops);
        if best[max_hops][src] <= amount_in {
            return None;
        }

        // Same pool-reuse fallback as `route_slippage_aware` -- see that
        // function's doc comment. `best[k][src] > amount_in` guarantees a
        // real profitable cycle exists at that depth (relaxation only
        // ever improves on the previous round's value, so this can't be
        // a stale carry-forward); depth strictly decreases every step
        // (see `widest_path`'s doc comment), so each reconstruction is
        // bounded by `max_hops` iterations without needing a separate
        // length safety guard.
        let mut path: Option<Vec<(usize, usize)>> = None;
        for k in (1..=max_hops).rev() {
            if best[k][src] <= amount_in {
                continue;
            }
            if let Some(candidate) = self.reconstruct_cycle(&pred, src, k) {
                if !path_reuses_a_pool(&candidate, &self.edges) {
                    path = Some(candidate);
                    break;
                }
            }
        }
        let path = path?;

        let mut hops = Vec::with_capacity(path.len());
        let mut amt = amount_in;
        for (u, ei) in &path {
            let edge = &self.edges[*u][*ei];
            let out = cp_quote(amt, edge.reserve_in, edge.reserve_out, edge.fee_bps);
            hops.push(Hop {
                pool_id: edge.pool_id,
                input_mint: edge.input_mint,
                output_mint: edge.output_mint,
                amount_in: amt,
                amount_out: out,
                dex: edge.dex,
            });
            amt = out;
            if amt == 0 {
                return None;
            }
        }
        if amt <= amount_in {
            return None;
        }
        Some(ArbitrageCycle { hops })
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Constant-product AMM quote: `dy = y * dx_after_fee / (x + dx_after_fee)`.
///
/// For Orca CLMM pools this is an approximation (valid near the current tick).
/// Returns 0 when any input is 0 or reserves are unknown.
pub(crate) fn cp_quote(amount_in: u64, reserve_in: u64, reserve_out: u64, fee_bps: u16) -> u64 {
    if reserve_in == 0 || reserve_out == 0 || amount_in == 0 {
        return 0;
    }
    let fee_mult = 10_000u64.saturating_sub(fee_bps as u64);
    let after_fee = amount_in.saturating_mul(fee_mult) / 10_000;
    let num = (after_fee as u128) * (reserve_out as u128);
    let den = (reserve_in as u128) + (after_fee as u128);
    (num / den) as u64
}

// ─── Tests ──────────────────────────────────────────────────────────────────
//
// Covers the shared `upsert_edge`/`remove_edge` primitives added for
// incremental per-pool router updates, exercised through `add_generic_pair`/
// `add_directed_edge` (plain-param functions, no dex-specific pool struct
// needed) plus one `add_orca_pool` test for the actual reordered/gate-fixed
// function this session's Orca CLMM fix and incremental-update refactor both
// touched. All synthetic `AccountId` literals per this codebase's
// established rule (never call `account_id_from_pubkey` from a test) --
// `seed_node` is private but reachable here since `mod tests` is nested
// inside this same module, so tests can build a `TradeRouter` directly
// without going through `from_router`'s `router::Router` dependency.
#[cfg(test)]
mod tests {
    use super::*;

    const MINT_A: AccountId = 1;
    const MINT_B: AccountId = 2;
    const MINT_C: AccountId = 3;
    const POOL_1: AccountId = 100;

    fn router_with_nodes(mints: &[AccountId]) -> TradeRouter {
        let mut r = TradeRouter::new();
        for &m in mints {
            r.seed_node(m);
        }
        r
    }

    #[test]
    fn note_pool_not_ready_false_until_threshold() {
        let mut r = TradeRouter::new();
        for _ in 0..(TradeRouter::POOL_NOT_READY_COOLDOWN_THRESHOLD - 1) {
            assert!(!r.note_pool_not_ready(POOL_1));
        }
    }

    #[test]
    fn note_pool_not_ready_true_at_threshold() {
        let mut r = TradeRouter::new();
        for _ in 0..(TradeRouter::POOL_NOT_READY_COOLDOWN_THRESHOLD - 1) {
            assert!(!r.note_pool_not_ready(POOL_1));
        }
        assert!(r.note_pool_not_ready(POOL_1));
    }

    #[test]
    fn note_pool_not_ready_resets_streak_after_threshold() {
        let mut r = TradeRouter::new();
        for _ in 0..TradeRouter::POOL_NOT_READY_COOLDOWN_THRESHOLD {
            r.note_pool_not_ready(POOL_1);
        }
        // Streak reset -- another full threshold's worth of failures is
        // needed before it fires again, not just one more call.
        for _ in 0..(TradeRouter::POOL_NOT_READY_COOLDOWN_THRESHOLD - 1) {
            assert!(!r.note_pool_not_ready(POOL_1));
        }
        assert!(r.note_pool_not_ready(POOL_1));
    }

    #[test]
    fn note_pool_not_ready_tracks_pools_independently() {
        let mut r = TradeRouter::new();
        const POOL_2: AccountId = 200;
        for _ in 0..(TradeRouter::POOL_NOT_READY_COOLDOWN_THRESHOLD - 1) {
            r.note_pool_not_ready(POOL_1);
        }
        // A different pool's own streak starts fresh, unaffected by
        // POOL_1's near-threshold count.
        assert!(!r.note_pool_not_ready(POOL_2));
    }

    #[test]
    fn mark_pool_not_ready_cooldown_doubles_each_cycle() {
        let mut r = TradeRouter::new();
        r.set_current_slot(0);
        r.mark_pool_not_ready_cooldown(POOL_1, 100);
        assert!(r.is_cooled_down(POOL_1));
        r.set_current_slot(99);
        assert!(r.is_cooled_down(POOL_1));
        r.set_current_slot(100);
        assert!(!r.is_cooled_down(POOL_1));

        // Second cycle: 100 * 2^1 = 200.
        r.set_current_slot(100);
        r.mark_pool_not_ready_cooldown(POOL_1, 100);
        r.set_current_slot(299);
        assert!(r.is_cooled_down(POOL_1));
        r.set_current_slot(300);
        assert!(!r.is_cooled_down(POOL_1));

        // Third cycle: 100 * 2^2 = 400.
        r.set_current_slot(300);
        r.mark_pool_not_ready_cooldown(POOL_1, 100);
        r.set_current_slot(699);
        assert!(r.is_cooled_down(POOL_1));
        r.set_current_slot(700);
        assert!(!r.is_cooled_down(POOL_1));
    }

    #[test]
    fn mark_pool_not_ready_cooldown_caps_out_at_max_cycles() {
        let mut r = TradeRouter::new();
        r.set_current_slot(0);
        for _ in 0..(TradeRouter::POOL_NOT_READY_MAX_COOLDOWN_CYCLES + 3) {
            r.mark_pool_not_ready_cooldown(POOL_1, 100);
        }
        let capped = 100u64 << (TradeRouter::POOL_NOT_READY_MAX_COOLDOWN_CYCLES - 1);
        r.set_current_slot(capped - 1);
        assert!(r.is_cooled_down(POOL_1));
        r.set_current_slot(capped);
        assert!(!r.is_cooled_down(POOL_1));
    }

    #[test]
    fn mark_pool_not_ready_cooldown_tracks_pools_independently() {
        let mut r = TradeRouter::new();
        const POOL_2: AccountId = 200;
        r.set_current_slot(0);
        r.mark_pool_not_ready_cooldown(POOL_1, 100);
        r.set_current_slot(100);
        r.mark_pool_not_ready_cooldown(POOL_1, 100);
        // POOL_2's first cycle is still the base duration (100 slots from
        // slot 100 -> cooled through slot 199), unaffected by POOL_1
        // already being on its second (escalated) cycle.
        r.mark_pool_not_ready_cooldown(POOL_2, 100);
        r.set_current_slot(199);
        assert!(r.is_cooled_down(POOL_2));
        r.set_current_slot(200);
        assert!(!r.is_cooled_down(POOL_2));
    }

    #[test]
    fn upsert_does_not_accumulate_duplicate_edges() {
        let mut r = router_with_nodes(&[MINT_A, MINT_B]);
        r.add_generic_pair(POOL_1, MINT_A, MINT_B, 2.0, 0.0, 100, 200, DexType::Sanctum);
        // Second call: same pool_id, larger reserves -- must replace the
        // first edge in place, not add a second parallel one.
        r.add_generic_pair(POOL_1, MINT_A, MINT_B, 2.0, 0.0, 1_000, 2_000, DexType::Sanctum);
        assert_eq!(r.out_degree(MINT_A), 1, "second call must replace, not accumulate");

        // Confirm the replacement actually took the second call's
        // reserves, not just that the count stayed at 1 -- cp_quote(10,
        // 100, 200, 0) == 18 (smaller reserves, more slippage) vs
        // cp_quote(10, 1000, 2000, 0) == 19 (larger reserves, less
        // slippage) -- a route at a fixed amount_in must reflect
        // whichever reserves are currently live.
        let route = r.route(MINT_A, MINT_B, 10, 1).expect("route should exist");
        assert_eq!(route.hops[0].amount_out, cp_quote(10, 1_000, 2_000, 0));
        assert_ne!(route.hops[0].amount_out, cp_quote(10, 100, 200, 0));
    }

    #[test]
    fn requote_edge_matches_cp_quote_at_a_different_amount() {
        let mut r = router_with_nodes(&[MINT_A, MINT_B]);
        r.add_generic_pair(POOL_1, MINT_A, MINT_B, 2.0, 0.003, 1_000_000, 2_000_000, DexType::Sanctum);
        let route = r.route(MINT_A, MINT_B, 1_000, 1).expect("route should exist");
        let hop = &route.hops[0];

        // requote_edge at the hop's own amount_in must reproduce the same
        // amount_out the router itself found.
        let same = r
            .requote_edge(hop.input_mint, hop.pool_id, hop.output_mint, hop.amount_in)
            .expect("edge should still exist");
        assert_eq!(same, hop.amount_out);

        // requote_edge at a different (corrected upstream) amount must
        // match a fresh cp_quote call at that amount, not the original.
        let corrected_in = hop.amount_in * 10;
        let corrected = r
            .requote_edge(hop.input_mint, hop.pool_id, hop.output_mint, corrected_in)
            .expect("edge should still exist");
        assert_eq!(corrected, cp_quote(corrected_in, 1_000_000, 2_000_000, 30));
        assert_ne!(corrected, hop.amount_out);
    }

    #[test]
    fn requote_edge_is_none_for_an_unknown_pool() {
        let r = router_with_nodes(&[MINT_A, MINT_B]);
        assert_eq!(r.requote_edge(MINT_A, POOL_1, MINT_B, 1_000), None);
    }

    #[test]
    fn cooled_down_pool_is_excluded_from_slippage_aware_search_until_it_expires() {
        let mut r = router_with_nodes(&[MINT_A, MINT_B]);
        r.add_generic_pair(POOL_1, MINT_A, MINT_B, 2.0, 0.0, 1_000_000, 2_000_000, DexType::Sanctum);
        r.set_current_slot(100);

        assert!(r.route_slippage_aware(MINT_A, MINT_B, 1_000, 1).is_some());

        r.mark_pool_cooldown(POOL_1, 50);
        assert!(
            r.route_slippage_aware(MINT_A, MINT_B, 1_000, 1).is_none(),
            "the only edge is on cooldown, no route should be found"
        );

        // Still within the cooldown window.
        r.set_current_slot(149);
        assert!(r.route_slippage_aware(MINT_A, MINT_B, 1_000, 1).is_none());

        // Cooldown expires at slot 150 (100 + 50) -- edge usable again.
        r.set_current_slot(150);
        assert!(r.route_slippage_aware(MINT_A, MINT_B, 1_000, 1).is_some());
    }

    #[test]
    fn invalidity_gate_removes_previously_valid_edge() {
        let mut r = router_with_nodes(&[MINT_A, MINT_B]);
        r.add_generic_pair(POOL_1, MINT_A, MINT_B, 2.0, 0.0, 100, 200, DexType::Sanctum);
        assert_eq!(r.out_degree(MINT_A), 1);
        assert!(r.route(MINT_A, MINT_B, 10, 1).is_some());

        // price_b_per_a <= 0.0 -- the same gate every add_* function uses
        // to signal "this pool is currently invalid" (drained reserve,
        // bad transient read, etc.).
        r.add_generic_pair(POOL_1, MINT_A, MINT_B, 0.0, 0.0, 100, 200, DexType::Sanctum);
        assert_eq!(r.out_degree(MINT_A), 0, "invalid gate must remove the stale edge, not just skip re-adding it");
        assert!(r.route(MINT_A, MINT_B, 10, 1).is_none());
    }

    #[test]
    fn shared_pool_id_removal_does_not_wipe_sibling_pairs() {
        // Mirrors Sanctum: every LST pair shares one pool_id
        // (pool_state_id), so upsert_edge/remove_edge must key on
        // (pool_id, output_mint), not pool_id alone.
        let mut r = router_with_nodes(&[MINT_A, MINT_B, MINT_C]);
        r.add_generic_pair(POOL_1, MINT_A, MINT_B, 2.0, 0.0, 100, 200, DexType::Sanctum);
        r.add_directed_edge(POOL_1, MINT_A, MINT_C, 5.0, 0.0, 100, 500, DexType::Sanctum);
        assert_eq!(r.out_degree(MINT_A), 2, "A->B and A->C must coexist under the same pool_id");

        // Re-upsert just the A->B pair -- must not disturb A->C.
        r.add_generic_pair(POOL_1, MINT_A, MINT_B, 2.5, 0.0, 100, 250, DexType::Sanctum);
        assert_eq!(r.out_degree(MINT_A), 2, "re-upserting one pair must not accumulate or remove the sibling");
        assert!(r.route(MINT_A, MINT_C, 10, 1).is_some(), "A->C must survive A->B's upsert");
    }

    #[test]
    fn directed_edge_upsert_and_removal_are_single_direction() {
        let mut r = router_with_nodes(&[MINT_A, MINT_B]);
        r.add_directed_edge(POOL_1, MINT_A, MINT_B, 1.5, 0.0, 100, 150, DexType::MarinadeLiquidUnstake);
        assert_eq!(r.out_degree(MINT_A), 1);
        assert_eq!(r.out_degree(MINT_B), 0, "add_directed_edge must not add a reverse edge");

        r.add_directed_edge(POOL_1, MINT_A, MINT_B, 0.0, 0.0, 100, 150, DexType::MarinadeLiquidUnstake);
        assert_eq!(r.out_degree(MINT_A), 0, "invalid gate must remove the one-directional edge");
    }

    #[test]
    fn orca_pool_going_invalid_removes_its_edges() {
        let pool = OrcaWhirlpool {
            token_mint_a: MINT_A,
            token_mint_b: MINT_B,
            vault_a: 900,
            vault_b: 901,
            liquidity: 1_000_000_000,
            sqrt_price_x64: 1u128 << 64, // spot_price() == 1.0
            tick_current_index: 0,
            tick_spacing: 64,
            fee_rate: 3000, // 0.30%
            reserve_a: 0,   // deliberately 0 -- no longer gates add_orca_pool
            reserve_b: 0,   // (see the fix earlier this session), unused here
        };
        let mut r = router_with_nodes(&[MINT_A, MINT_B]);
        r.add_orca_pool(POOL_1, &pool, DexType::OrcaWhirlpool);
        assert_eq!(r.out_degree(MINT_A), 1);
        assert_eq!(r.out_degree(MINT_B), 1);

        let mut drained = pool;
        drained.liquidity = 0; // genuinely drained pool
        r.add_orca_pool(POOL_1, &drained, DexType::OrcaWhirlpool);
        assert_eq!(r.out_degree(MINT_A), 0, "a drained pool's stale edges must be removed, not left behind");
        assert_eq!(r.out_degree(MINT_B), 0);
    }

    #[test]
    fn orca_pool_with_overflowing_virtual_reserve_is_rejected_not_corrupted() {
        // Exact liquidity/sqrt_price_x64 from a real live pool
        // (438905200-era, pool 360348689) whose true virtual_a (L/sqrtP)
        // is ~4.0e20 -- ~21.7x past u64::MAX. Before the fix,
        // virtual_reserves() saturated this to u64::MAX (a plausible-
        // looking but ~21.7x-too-small number) instead of rejecting it,
        // which fed cp_quote a corrupted reserve and produced a quote
        // whose implied price differed by ~2394x (and flipped direction)
        // between a 100 SOL and a 0.01 SOL probe through the same pool --
        // not real slippage, a genuinely wrong number.
        let pool = OrcaWhirlpool {
            token_mint_a: MINT_A,
            token_mint_b: MINT_B,
            vault_a: 900,
            vault_b: 901,
            liquidity: 26_837_485_638_526_565_377,
            sqrt_price_x64: 1_237_536_819_238_404_774,
            tick_current_index: -54038,
            tick_spacing: 64,
            fee_rate: 100,
            reserve_a: 0,
            reserve_b: 0,
        };
        // virtual_a = L/sqrtP and virtual_b = L*sqrtP are inversely
        // related in sqrtP -- a small sqrtP (this pool's, ~0.067) can
        // overflow virtual_a while leaving virtual_b (a real, in-range
        // ~1.8e18) untouched. Only virtual_a is expected to be rejected
        // here; add_orca_pool's `virtual_a == 0 || virtual_b == 0` OR-gate
        // is what makes that single overflow enough to reject the whole
        // pool below, not a claim that virtual_b overflows too.
        let (virtual_a, virtual_b) = pool.virtual_reserves();
        assert_eq!(virtual_a, 0, "an overflowing virtual_a must be rejected (0), not saturated to a wrong-but-plausible u64::MAX");
        assert_ne!(virtual_b, 0, "virtual_b genuinely fits in u64 here -- this test is about virtual_a's overflow specifically");

        let mut r = router_with_nodes(&[MINT_A, MINT_B]);
        r.add_orca_pool(POOL_1, &pool, DexType::OrcaWhirlpool);
        assert_eq!(r.out_degree(MINT_A), 0, "add_orca_pool's OR-gate must reject the whole pool when either virtual reserve overflows, not just half-insert it");
        assert_eq!(r.out_degree(MINT_B), 0);
    }

    #[test]
    fn route_slippage_aware_prefers_deep_liquidity_over_better_spot_price() {
        // Two parallel A->B pools: "thin" has a better spot price (2.0)
        // but reserves far too small for a 100_000-unit trade; "deep" has
        // a worse spot price (1.5) but reserves large enough to absorb it
        // with minimal slippage. route() (spot-price-only) always prefers
        // "thin"; route_slippage_aware must prefer "deep" at this size
        // since it actually outputs more.
        const POOL_THIN: AccountId = 101;
        const POOL_DEEP: AccountId = 102;
        let mut r = router_with_nodes(&[MINT_A, MINT_B]);
        r.add_directed_edge(POOL_THIN, MINT_A, MINT_B, 2.0, 0.0, 1_000, 2_000, DexType::Sanctum);
        r.add_directed_edge(POOL_DEEP, MINT_A, MINT_B, 1.5, 0.0, 10_000_000, 15_000_000, DexType::Sanctum);

        let amount_in = 100_000;
        let spot_route = r.route(MINT_A, MINT_B, amount_in, 1).expect("route should exist");
        assert_eq!(spot_route.hops[0].pool_id, POOL_THIN, "route() must pick the better-spot-price pool regardless of size");

        let real_route = r
            .route_slippage_aware(MINT_A, MINT_B, amount_in, 1)
            .expect("route_slippage_aware should exist");
        assert_eq!(real_route.hops[0].pool_id, POOL_DEEP, "route_slippage_aware must pick the pool that actually outputs more at this size");
        assert!(
            real_route.amount_out() > spot_route.amount_out(),
            "the slippage-aware pick ({}) must genuinely outperform the spot-price pick ({}) at this size",
            real_route.amount_out(),
            spot_route.amount_out(),
        );
    }

    #[test]
    fn route_slippage_aware_rejects_a_path_that_revisits_a_node_via_distinct_pools() {
        // Live-confirmed real shape (real sell-direction quote): a direct
        // A->B pool gives a sane quote, but widest_path's DP finds a
        // "better" 3-hop path A->B->C->B that loops back through B via
        // two *other*, distinct pools (no single pool reused, so
        // `path_reuses_a_pool` alone doesn't catch this) -- one of which
        // (B->C) is nearly drained, producing a wildly inflated output.
        // route_slippage_aware must fall back to the sane direct route
        // rather than return the nonsensical node-revisiting one. Real
        // magnitudes (scaled from the live incident, not arbitrary).
        const POOL_DIRECT: AccountId = 101;
        const POOL_B_TO_C: AccountId = 102;
        const POOL_C_TO_B: AccountId = 103;
        let mut r = router_with_nodes(&[MINT_A, MINT_B, MINT_C]);
        r.add_directed_edge(POOL_DIRECT, MINT_A, MINT_B, 45_126_385_660.0 / 409_379_729_940.0, 0.003, 409_379_729_940, 45_126_385_660, DexType::Sanctum);
        r.add_directed_edge(POOL_B_TO_C, MINT_B, MINT_C, 16_284_975_681.0 / 22_315_675.0, 0.008, 22_315_675, 16_284_975_681, DexType::Sanctum);
        r.add_directed_edge(POOL_C_TO_B, MINT_C, MINT_B, 7_064_815_085_947.0 / 71_425_651_518_255.0, 0.0025, 71_425_651_518_255, 7_064_815_085_947, DexType::Sanctum);

        let amount_in = 115_247_767;
        let route = r.route_slippage_aware(MINT_A, MINT_B, amount_in, 3).expect("a sane route should exist");
        assert_eq!(
            route.hops.len(),
            1,
            "expected the sane direct route, got a {}-hop route -- likely the node-revisiting one",
            route.hops.len(),
        );
        assert_eq!(route.hops[0].pool_id, POOL_DIRECT);
        assert!(
            route.amount_out() < amount_in * 2,
            "output {} looks like the inflated node-revisiting quote (~58x input), not the sane direct one",
            route.amount_out(),
        );
    }

    #[test]
    fn widest_path_excludes_a_hop_that_trades_too_large_a_fraction_of_its_own_pool() {
        // Live-confirmed real shape, distinct from the node-revisit case
        // above: a genuinely non-revisiting 2-hop path A->C->B (no token
        // touched twice, so `path_revisits_a_node` doesn't catch this)
        // still quotes a wildly inflated output because its first hop
        // trades ~91% of a shallow pool's own reserve_in -- a real
        // `cp_quote` artifact, not genuine depth. Without the per-hop
        // pool-utilization cap this 2-hop path's raw output (192_365)
        // beats the deep direct pool's sane one (9_900), so
        // `route_slippage_aware` must exclude the shallow hop and fall
        // back to the direct route.
        const POOL_DIRECT: AccountId = 201;
        const POOL_A_TO_C: AccountId = 202;
        const POOL_C_TO_B: AccountId = 203;
        let mut r = router_with_nodes(&[MINT_A, MINT_B, MINT_C]);
        r.add_directed_edge(POOL_DIRECT, MINT_A, MINT_B, 1.0, 0.0, 1_000_000, 1_000_000, DexType::Sanctum);
        r.add_directed_edge(POOL_A_TO_C, MINT_A, MINT_C, 100_000_000.0 / 11_000.0, 0.0, 11_000, 100_000_000, DexType::Sanctum);
        r.add_directed_edge(POOL_C_TO_B, MINT_C, MINT_B, 1_000_000.0 / 200_000_000.0, 0.0, 200_000_000, 1_000_000, DexType::Sanctum);

        let amount_in = 10_000;
        let route = r.route_slippage_aware(MINT_A, MINT_B, amount_in, 2).expect("a sane route should exist");
        assert_eq!(
            route.hops.len(),
            1,
            "expected the sane direct route, got a {}-hop route -- likely the thin-pool inflated-quote artifact",
            route.hops.len(),
        );
        assert_eq!(route.hops[0].pool_id, POOL_DIRECT);
    }

    #[test]
    fn find_arbitrage_slippage_aware_finds_a_cycle_find_arbitrage_misses() {
        // Same "thin, good spot price" vs "deep, worse spot price" A->B
        // setup, plus a real (negligible-slippage) B->A return leg.
        // Spot-price product through THIN is 2.0*1.0=2.0 (looks great),
        // but a 100_000-unit trade craters against its tiny reserves and
        // comes back a net loss. Spot-price product through DEEP is only
        // 1.5*1.0=1.5 (looks worse), but with negligible slippage at this
        // size it's a genuine, real profit. find_arbitrage can only ever
        // consider the one cycle Bellman-Ford picks by spot rate (THIN)
        // and correctly rejects it as unprofitable after slippage --
        // but that means it misses the real opportunity through DEEP
        // entirely. find_arbitrage_slippage_aware must find it.
        const POOL_THIN: AccountId = 101;
        const POOL_DEEP: AccountId = 102;
        const POOL_RETURN: AccountId = 103;
        let mut r = router_with_nodes(&[MINT_A, MINT_B]);
        r.add_directed_edge(POOL_THIN, MINT_A, MINT_B, 2.0, 0.0, 1_000, 2_000, DexType::Sanctum);
        r.add_directed_edge(POOL_DEEP, MINT_A, MINT_B, 1.5, 0.0, 10_000_000, 15_000_000, DexType::Sanctum);
        r.add_directed_edge(POOL_RETURN, MINT_B, MINT_A, 1.0, 0.0, 1_000_000_000, 1_000_000_000, DexType::Sanctum);

        let amount_in = 100_000;
        assert!(
            r.find_arbitrage(amount_in).is_none(),
            "find_arbitrage must reject the only cycle it's capable of considering (THIN, spot-optimal but craters at this size)"
        );

        let cycle = r
            .find_arbitrage_slippage_aware(MINT_A, amount_in, 2)
            .expect("find_arbitrage_slippage_aware should find the real opportunity through DEEP");
        assert_eq!(cycle.hops[0].pool_id, POOL_DEEP, "must route the outbound leg through the deep pool, not the spot-optimal thin one");
        assert!(cycle.profit_raw() > 0, "the found cycle must be genuinely profitable");
    }

    #[test]
    fn net_profit_lamports_subtracts_real_transaction_cost() {
        let cycle = ArbitrageCycle {
            hops: vec![Hop {
                pool_id: POOL_1,
                input_mint: MINT_A,
                output_mint: MINT_B,
                amount_in: 1_000_000,
                amount_out: 1_050_000, // gross profit: 50_000 lamports
                dex: DexType::Sanctum,
            }],
        };
        // base_fee = 1 * 5000 = 5000; priority_fee = 200_000 cu * 10_000
        // micro-lamports/cu / 1_000_000 = 2000. Total cost = 7000.
        let net = cycle.net_profit_lamports(200_000, 10_000, 1);
        assert_eq!(net, 50_000 - 7_000);
    }

    #[test]
    fn net_profit_lamports_can_go_negative_for_a_thin_cycle() {
        let cycle = ArbitrageCycle {
            hops: vec![Hop {
                pool_id: POOL_1,
                input_mint: MINT_A,
                output_mint: MINT_B,
                amount_in: 1_000_000,
                amount_out: 1_001_000, // gross profit: only 1000 lamports
                dex: DexType::Sanctum,
            }],
        };
        // Same cost as above (7000) comfortably exceeds the 1000-lamport
        // gross profit -- must report a real negative number, not
        // saturate at 0.
        let net = cycle.net_profit_lamports(200_000, 10_000, 1);
        assert_eq!(net, 1_000 - 7_000);
        assert!(net < 0);
    }
}
