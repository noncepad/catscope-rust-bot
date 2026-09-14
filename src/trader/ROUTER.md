# High-Performance Liquidity-Partitioned Rust Trading Router

This document outlines the production architecture for a 5,000-token Solana trading router implemented in Rust. The design optimizes routing latency and path-finding throughput by partitioning a sparse graph into a hierarchical, SIMD-friendly structure based on liquidity depth.

---

## 1. Structural Overview & Liquidity Tiers

To scale past the $O(V^3)$ bottleneck of 5,000 tokens, the network topology is decomposed into three distinct structural layers. This eliminates brute-force graph traversal and aligns memory layouts for hardware-accelerated routing.

### Tier 1: The Sovereign Core (Ultra-Dense Clique)

* **Assets:** `USDC`, `SOL`, `JITO`, `mSOL`, `USDT`.
* **Characteristics:** Extremely deep liquidity pools exist between virtually all pairs in this group.
* **Design:** Forms a permanent, fully dense $5 \times 5$ matrix that acts as the primary highway and universal settlement backbone.

### Tier 2: Liquid Clusters (Dense Subgraphs)

* **Assets:** High-to-medium liquidity tokens (e.g., `JUP`, `BONK`, `WIF`, `PYUSD`).
* **Characteristics:** Assets possessing a liquidity degree of $\ge 2$ across the minimum USD threshold. They connect reliably to multiple Tier 1 assets.
* **Design:** Grouped into fixed-size dense matrices (e.g., 64 elements) to facilitate loop unrolling and SIMD Min-Plus matrix multiplication.

### Tier 3: Long-Tail Spokes (Isolated Trees)

* **Assets:** Low-liquidity, exotic, or newly launched tokens.
* **Characteristics:** Niche assets that maintain exactly *one* liquid pool, almost universally paired against `SOL` or `USDC`. Secondary pools are illiquid and present excessive slippage.
* **Design:** Completely excluded from matrix calculations. Stored in a flat, cache-friendly $O(1)$ lookup array pointing directly to their parent Tier 1 or Tier 2 hub.

---

## 2. Partitioning Algorithm

The partitioning pipeline is executed asynchronously (e.g., every epoch or hour) to update graph topology based on macro liquidity shifts. On a per-tick or per-block basis, only the prices inside the resulting dense matrices are modified.

```rust
pub struct Pool {
    pub token_a: usize,
    pub token_b: usize,
    pub liquidity_usd: f64,
    pub price_a_to_b: f64,
}

const CORE_TOKENS: [usize; 5] = [0, 1, 2, 3, 4]; // USDC, SOL, JITO, mSOL, USDT
const MIN_CLUSTER_LIQUIDITY: f64 = 50_000.0;     // Target threshold for Tier 2 entry

pub struct LiquidityPartitioner {
    pub token_degrees: Vec<usize>,
    pub primary_hub: Vec<Option<usize>>, // Tier 3 -> Tier 1/2 mapping
    pub tier2_clusters: Vec<Vec<usize>>, // Array of clustered token IDs
}

impl LiquidityPartitioner {
    pub fn new(total_tokens: usize) -> Self {
        Self {
            token_degrees: vec![0; total_tokens],
            primary_hub: vec![None; total_tokens],
            tier2_clusters: Vec::new(),
        }
    }

    pub fn partition(&mut self, all_pools: &[Pool], total_tokens: usize) {
        // 1. Reset metrics
        self.token_degrees.fill(0);
        self.primary_hub.fill(None);

        // 2. Compute high-liquidity degrees
        for pool in all_pools {
            if pool.liquidity_usd >= MIN_CLUSTER_LIQUIDITY {
                self.token_degrees[pool.token_a] += 1;
                self.token_degrees[pool.token_b] += 1;
            }
        }

        // 3. Extract Tier 3 Spokes (Degree <= 1)
        for token_id in 0..total_tokens {
            if CORE_TOKENS.contains(&token_id) { continue; }
            
            if self.token_degrees[token_id] <= 1 {
                let best_pool = all_pools.iter()
                    .filter(|p| p.token_a == token_id || p.token_b == token_id)
                    .max_by(|a, b| a.liquidity_usd.partial_cmp(&b.liquidity_usd).unwrap());
                    
                if let Some(pool) = best_pool {
                    let hub = if pool.token_a == token_id { pool.token_b } else { pool.token_a };
                    self.primary_hub[token_id] = Some(hub);
                }
            }
        }
        
        // 4. Batch remaining Tier 2 tokens into SIMD-friendly clusters (e.g., sizes of 64)
        // (Implementation chunks remaining high-degree tokens alongside Core 5 anchors)
    }
}
