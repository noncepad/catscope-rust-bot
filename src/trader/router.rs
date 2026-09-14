use core::f32;
use std::{collections::HashMap, hash::BuildHasherDefault};
use twox_hash::XxHash64;

use crate::graph::AccountId;

type Xx = BuildHasherDefault<XxHash64>;

// ─── USD trade-size buckets ────────────────────────────────────────────────

const BUCKET_LIST: &[f32] = &[
    100.0, 500.0, 1_000.0, 5_000.0, 10_000.0, 50_000.0, 100_000.0, 500_000.0,
];
const BUCKET_COUNT: usize = BUCKET_LIST.len();

// ─── Tier constants ────────────────────────────────────────────────────────

/// Tier 1: USDC, SOL, JITO, mSOL, USDT.
/// These are the first five indices assigned in `m_mint` (register them first).
const CORE_SIZE: usize = 5;
const CORE_TOKENS: [usize; CORE_SIZE] = [0, 1, 2, 3, 4];

/// Tier 2 cluster width — sized for SIMD-friendly loop unrolling.
const CLUSTER_SIZE: usize = 64;

/// Minimum pool liquidity (USD) for a token to qualify as Tier 1 or Tier 2.
const MIN_CLUSTER_LIQUIDITY: f64 = 50_000.0;

const INF: f32 = f32::INFINITY;
const NO_HOP: u16 = u16::MAX;

// ─── Pool snapshot used by the partitioner ─────────────────────────────────

/// Lightweight pool snapshot for the partitioning pass.
/// `token_a` and `token_b` are global token indices from `Router::m_mint`.
pub struct Pool {
    pub token_a: usize,
    pub token_b: usize,
    pub liquidity_usd: f64,
    pub price_a_to_b: f64,
}

// ─── Liquidity partitioner ─────────────────────────────────────────────────

/// Classifies every token into one of three tiers based on on-chain liquidity.
///
/// Run this asynchronously (per epoch / per hour).  On a per-block basis only
/// the dense-matrix prices need to be refreshed.
#[derive(Debug)]
pub struct LiquidityPartitioner {
    /// Number of pools with `liquidity_usd >= MIN_CLUSTER_LIQUIDITY` touching each token.
    pub token_degrees: Vec<usize>,
    /// Tier 3 spoke → Tier 1/2 hub global index.
    pub primary_hub: Vec<Option<usize>>,
    /// Tier 2 clusters; each inner `Vec` holds up to `CLUSTER_SIZE` global indices.
    pub tier2_clusters: Vec<Vec<usize>>,
}

impl LiquidityPartitioner {
    pub fn new(total_tokens: usize) -> Self {
        Self {
            token_degrees: vec![0; total_tokens],
            primary_hub: vec![None; total_tokens],
            tier2_clusters: Vec::new(),
        }
    }

    /// Reclassify all tokens from a pool snapshot.
    pub fn partition(&mut self, all_pools: &[Pool], total_tokens: usize) {
        // 1. Reset
        self.token_degrees.resize(total_tokens, 0);
        self.token_degrees.fill(0);
        self.primary_hub.resize(total_tokens, None);
        self.primary_hub.fill(None);
        self.tier2_clusters.clear();

        // 2. Compute high-liquidity degrees
        for pool in all_pools {
            if pool.liquidity_usd >= MIN_CLUSTER_LIQUIDITY {
                self.token_degrees[pool.token_a] += 1;
                self.token_degrees[pool.token_b] += 1;
            }
        }

        // 3. Assign Tier 3 spokes: non-core tokens whose high-liquidity degree ≤ 1
        for token_id in 0..total_tokens {
            if CORE_TOKENS.contains(&token_id) {
                continue;
            }
            if self.token_degrees[token_id] <= 1 {
                // total_cmp (not partial_cmp().unwrap()) so a NaN
                // liquidity_usd -- e.g. from the build-time anchor-price
                // BFS overflowing to inf on some extreme-ratio real pool,
                // then inf * 0.0 on a later hop -- can't panic here. NaN
                // sorts as the total-cmp minimum, so a NaN-liquidity pool
                // just never wins "best", same as build.rs already clamps
                // non-finite liquidity_usd to 0.0 (unpriced) before this
                // data is even embedded.
                let best = all_pools
                    .iter()
                    .filter(|p| p.token_a == token_id || p.token_b == token_id)
                    .max_by(|a, b| a.liquidity_usd.total_cmp(&b.liquidity_usd));
                if let Some(pool) = best {
                    let hub = if pool.token_a == token_id {
                        pool.token_b
                    } else {
                        pool.token_a
                    };
                    self.primary_hub[token_id] = Some(hub);
                }
            }
        }

        // 4. Batch remaining Tier 2 tokens into CLUSTER_SIZE-wide groups.
        //    Tier 2 = non-core, non-spoke (degree > 1).
        let tier2: Vec<usize> = (0..total_tokens)
            .filter(|&t| !CORE_TOKENS.contains(&t) && self.primary_hub[t].is_none())
            .collect();
        self.tier2_clusters = tier2
            .chunks(CLUSTER_SIZE)
            .map(|chunk| chunk.to_vec())
            .collect();
    }
}

// ─── Router ────────────────────────────────────────────────────────────────

/// Three-tier trading router.
///
/// # Typical lifecycle
///
/// 1. Register the five core mints first via [`register_mint`] so they receive
///    global indices 0–4, matching `CORE_TOKENS`.
/// 2. Register all other mints.
/// 3. Periodically call [`rebuild_partitions`] with a pool-liquidity snapshot
///    to reclassify tokens into tiers and rebuild cluster matrices.
/// 4. On every price tick call [`update`] for each pool that changed.
/// 5. After each batch of updates call [`floyd_warshall`].
/// 6. Query routes with [`best_route`].
/// Snapshot of the current tier partitioning, see [`Router::stats`].
#[derive(Debug)]
pub struct RouterStats {
    pub token_count: usize,
    /// Always CORE_SIZE (5) -- included for symmetry with the other tiers.
    pub tier1_count: usize,
    pub tier2_cluster_count: usize,
    pub tier2_token_count: usize,
    pub tier3_spoke_count: usize,
}

impl std::fmt::Display for RouterStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "tokens={} tier1={} tier2_clusters={} tier2_tokens={} tier3_spokes={}",
            self.token_count,
            self.tier1_count,
            self.tier2_cluster_count,
            self.tier2_token_count,
            self.tier3_spoke_count,
        )
    }
}

#[derive(Debug)]
pub struct Router {
    /// Partitioner — owns tier assignments.
    pub partitioner: LiquidityPartitioner,
    /// Mint → global token index.
    m_mint: HashMap<AccountId, usize, Xx>,
    /// Next unassigned global index.
    index: usize,
    lambda: f32,
    /// Tier 1: one `Bucket` per USD level, fixed `CORE_SIZE × CORE_SIZE`.
    core: Vec<Bucket>,
    /// Tier 2: one `ClusterMatrix` per cluster.
    clusters: Vec<ClusterMatrix>,
}

impl Router {
    pub fn new(token_count: usize, lambda: f32) -> Self {
        let core = (0..BUCKET_COUNT)
            .map(|_| Bucket::new(CORE_SIZE, lambda))
            .collect();
        Self {
            partitioner: LiquidityPartitioner::new(token_count),
            m_mint: HashMap::with_capacity_and_hasher(token_count, Default::default()),
            index: 0,
            lambda,
            core,
            clusters: Vec::new(),
        }
    }

    /// Assign a global index to `mint` if not already known; return the index.
    pub fn register_mint(&mut self, mint: AccountId) -> usize {
        if let Some(&ix) = self.m_mint.get(&mint) {
            return ix;
        }
        let ix = self.index;
        self.index = self.index.saturating_add(1);
        self.m_mint.insert(mint, ix);
        ix
    }

    /// Every mint this Router has classified (Tier1 core, Tier2 cluster, or
    /// Tier3 spoke via `primary_hub`) -- the liquidity-vetted universe
    /// `TradeRouter::from_router` seeds its node set from, so a live pool
    /// touching a mint this Router has never seen (i.e. never made it into
    /// the build-time ROUTER_POOLS snapshot with real liquidity) can't
    /// enter the live trade graph either.
    pub fn known_mints(&self) -> impl Iterator<Item = AccountId> + '_ {
        self.m_mint.keys().copied()
    }

    /// Snapshot of the current tier partitioning -- for the periodic
    /// "price graph stats" log (see StateHelper::start in
    /// brain/arbv1/state.rs). `partitioner`'s own fields (`token_degrees`,
    /// `primary_hub`, `tier2_clusters`) are already public, this just adds
    /// what only `Router` itself knows (total registered tokens, and the
    /// materialized `ClusterMatrix` count, which can lag
    /// `tier2_clusters.len()` between a `register_mint` batch and the next
    /// `rebuild_partitions`).
    pub fn stats(&self) -> RouterStats {
        let tier2_tokens = self.partitioner.tier2_clusters.iter().map(Vec::len).sum();
        let tier3_spokes = self.partitioner.primary_hub.iter().filter(|h| h.is_some()).count();
        RouterStats {
            token_count: self.index,
            tier1_count: CORE_SIZE,
            tier2_cluster_count: self.clusters.len(),
            tier2_token_count: tier2_tokens,
            tier3_spoke_count: tier3_spokes,
        }
    }

    /// Reclassify tokens and rebuild cluster matrices from a liquidity snapshot.
    ///
    /// Cheap to call in an async background task; the dense-matrix writes in
    /// [`update`] are the hot path.
    pub fn rebuild_partitions(&mut self, all_pools: &[Pool]) {
        let total = self.index;
        self.partitioner.partition(all_pools, total);
        self.clusters = self
            .partitioner
            .tier2_clusters
            .iter()
            .map(|tokens| ClusterMatrix::new(tokens.clone(), self.lambda))
            .collect();
    }

    /// Update edge weights for a pool at every USD bucket level.
    ///
    /// Mints not yet in `m_mint` are silently skipped; call [`register_mint`]
    /// first.
    pub fn update<P: SwapPricer>(&mut self, pool: &P) {
        let (mint_a, mint_b) = pool.pair();
        let ix_a = match self.m_mint.get(&mint_a) {
            Some(&x) => x,
            None => return,
        };
        let ix_b = match self.m_mint.get(&mint_b) {
            Some(&x) => x,
            None => return,
        };
        let pool_id = pool.pool_id();

        for k in 0..BUCKET_COUNT {
            let (inp_ab, out_ab) = pool.reference_price(BucketReferenceLevel::AtoB(k));
            let (inp_ba, out_ba) = pool.reference_price(BucketReferenceLevel::BtoA(k));

            // Tier 1: both mints are core tokens
            if let (Some(la), Some(lb)) = (core_local(ix_a), core_local(ix_b)) {
                self.core[k].update_pool(la, lb, pool_id, inp_ab, out_ab);
                self.core[k].update_pool(lb, la, pool_id, inp_ba, out_ba);
                continue;
            }

            // Tier 2: both mints reside in the same cluster
            for cm in &mut self.clusters {
                if let (Some(la), Some(lb)) = (cm.local_ix(ix_a), cm.local_ix(ix_b)) {
                    cm.buckets[k].update_pool(la, lb, pool_id, inp_ab, out_ab);
                    cm.buckets[k].update_pool(lb, la, pool_id, inp_ba, out_ba);
                    break;
                }
            }
            // Tier 3 spokes: no dense matrix — routing is spoke→hub→core→hub→spoke
        }
    }

    /// Run Floyd-Warshall on all dense matrices to refresh all-pairs paths.
    pub fn floyd_warshall(&mut self) {
        for b in &mut self.core {
            b.floyd_warshall();
        }
        for cm in &mut self.clusters {
            for b in &mut cm.buckets {
                b.floyd_warshall();
            }
        }
    }

    /// Reset edge weights in all dense matrices (call before re-inserting prices).
    pub fn clear_edges(&mut self) {
        for b in &mut self.core {
            b.clear_edges();
        }
        for cm in &mut self.clusters {
            for b in &mut cm.buckets {
                b.clear_edges();
            }
        }
    }

    /// Return the best route between two mints at a given USD bucket index.
    ///
    /// Returns `None` when no path exists within the same tier matrix.
    pub fn best_route(
        &self,
        src: AccountId,
        dst: AccountId,
        bucket: usize,
    ) -> Option<Vec<RouteHop>> {
        let &ix_a = self.m_mint.get(&src)?;
        let &ix_b = self.m_mint.get(&dst)?;

        // Both core?
        if let (Some(la), Some(lb)) = (core_local(ix_a), core_local(ix_b)) {
            return self.core[bucket].best_route_hops(la, lb);
        }

        // Same Tier 2 cluster?
        for cm in &self.clusters {
            if let (Some(la), Some(lb)) = (cm.local_ix(ix_a), cm.local_ix(ix_b)) {
                return cm.buckets[bucket].best_route_hops(la, lb);
            }
        }

        // Spoke routing (Tier 3) deferred: resolve hub then re-enter core matrix
        None
    }

    /// Resolve the Tier 3 hub for a spoke mint, if any.
    pub fn spoke_hub(&self, mint: AccountId) -> Option<AccountId> {
        let &ix = self.m_mint.get(&mint)?;
        let hub_ix = self.partitioner.primary_hub.get(ix)?.as_ref()?;
        self.m_mint
            .iter()
            .find(|(_, &v)| v == *hub_ix)
            .map(|(&k, _)| k)
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────

#[inline]
fn core_local(global: usize) -> Option<usize> {
    if global < CORE_SIZE {
        Some(global)
    } else {
        None
    }
}

// ─── Cluster matrix ────────────────────────────────────────────────────────

#[derive(Debug)]
struct ClusterMatrix {
    /// Global token indices in this cluster; position = cluster-local index.
    tokens: Vec<usize>,
    /// One `Bucket` per USD level.
    buckets: Vec<Bucket>,
}

impl ClusterMatrix {
    fn new(tokens: Vec<usize>, lambda: f32) -> Self {
        let size = tokens.len();
        let buckets = (0..BUCKET_COUNT)
            .map(|_| Bucket::new(size, lambda))
            .collect();
        Self { tokens, buckets }
    }

    #[inline]
    fn local_ix(&self, global: usize) -> Option<usize> {
        self.tokens.iter().position(|&t| t == global)
    }
}

// ─── SwapPricer / BucketReferenceLevel ────────────────────────────────────

pub trait SwapPricer {
    fn pool_id(&self) -> AccountId;
    fn pair(&self) -> (AccountId, AccountId);
    /// Return `(input_amount, output_amount)` for the given USD bucket level.
    fn reference_price(&self, level: BucketReferenceLevel) -> (f32, f32);
}

#[derive(Debug, Clone, Copy)]
pub enum BucketReferenceLevel {
    /// Spend token A, receive token B.
    AtoB(usize),
    /// Spend token B, receive token A.
    BtoA(usize),
}

pub struct RouteHop {
    pub from: usize,
    pub to: usize,
    pub pool: AccountId,
}

// ─── Dense cost matrix (Floyd-Warshall) ────────────────────────────────────

#[derive(Debug)]
struct Bucket {
    size: usize,
    /// Negative-log edge weights; `cost[i*size+j]` = cost from i to j.
    cost: Box<[f32]>,
    /// Next-hop table for path reconstruction.
    next: Box<[u16]>,
    /// Best pool for each direct edge.
    best_pool: Box<[AccountId]>,
    lambda: f32,
}

impl Bucket {
    fn new(size: usize, lambda: f32) -> Self {
        let n = size * size;
        let mut cost = vec![INF; n].into_boxed_slice();
        let mut next = vec![NO_HOP; n].into_boxed_slice();
        for i in 0..size {
            cost[i * size + i] = 0.0;
            next[i * size + i] = i as u16;
        }
        let best_pool = vec![0; n].into_boxed_slice();
        Self {
            size,
            cost,
            next,
            best_pool,
            lambda,
        }
    }

    #[inline(always)]
    fn idx(&self, src: usize, dst: usize) -> usize {
        src * self.size + dst
    }

    fn update_pool(
        &mut self,
        src: usize,
        dst: usize,
        pool_id: AccountId,
        input_amount: f32,
        output_amount: f32,
    ) {
        let rate = output_amount / input_amount;
        // Negative-log transform so Bellman-Ford/FW minimises cost
        let weight = -rate.ln() + self.lambda * (input_amount / output_amount).powi(2);
        let idx = self.idx(src, dst);
        if weight < self.cost[idx] {
            self.cost[idx] = weight;
            self.best_pool[idx] = pool_id;
            self.next[idx] = dst as u16;
        }
    }

    fn clear_edges(&mut self) {
        for i in 0..self.size {
            for j in 0..self.size {
                self.cost[i * self.size + j] = if i == j { 0.0 } else { INF };
            }
        }
    }

    fn floyd_warshall(&mut self) {
        for k in 0..self.size {
            for i in 0..self.size {
                let ik = self.cost[i * self.size + k];
                if ik == INF {
                    continue;
                }
                for j in 0..self.size {
                    let idx_ij = i * self.size + j;
                    let alt = ik + self.cost[k * self.size + j];
                    if alt < self.cost[idx_ij] {
                        self.cost[idx_ij] = alt;
                        self.next[idx_ij] = self.next[i * self.size + k];
                    }
                }
            }
        }
    }

    fn best_route(&self, src: usize, dst: usize) -> Option<Vec<usize>> {
        if self.next[src * self.size + dst] == NO_HOP {
            return None;
        }
        let mut route = Vec::with_capacity(8);
        route.push(src);
        let mut cur = src;
        while cur != dst {
            cur = self.next[cur * self.size + dst] as usize;
            route.push(cur);
            if route.len() > self.size {
                return None; // cycle guard
            }
        }
        Some(route)
    }

    fn best_route_hops(&self, src: usize, dst: usize) -> Option<Vec<RouteHop>> {
        let path = self.best_route(src, dst)?;
        Some(
            path.windows(2)
                .map(|w| RouteHop {
                    from: w[0],
                    to: w[1],
                    pool: self.best_pool[w[0] * self.size + w[1]],
                })
                .collect(),
        )
    }
}
