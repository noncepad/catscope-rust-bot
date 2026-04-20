Goal: Implement an ultra-low-latency arbitrage detector in Rust that models markets as a directed weighted graph and detects negative cycles incrementally when prices update.

Concept:

* Nodes represent assets (or asset@venue).
* Directed edges represent tradable conversions.
* Edge weight uses a log transform so multiplication becomes addition.

Edge weight formula:
w_ij = -ln(price_ij)

A profitable arbitrage cycle exists when:
sum(edge_weights) < 0

Profit calculation:
profit = exp(-cycle_weight_sum) - 1

Design constraints:

* Extremely low latency (sub-microsecond update path).
* No heap allocations in the hot path.
* Cache-friendly memory layout.
* Incremental updates only.

---

## Core Rust Data Structures

Use index-based arrays instead of hash maps.

```rust
type EdgeId = u32;
type CycleId = u32;

struct Edge {
    weight: f64,
}

struct Graph {
    edges: Vec<Edge>,

    // edge → cycles containing this edge
    edge_to_cycles: Vec<Vec<CycleId>>,

    // flattened cycle storage
    cycle_edges: Vec<EdgeId>,
    cycle_offsets: Vec<usize>,

    // cached cycle cost
    cycle_cost: Vec<f64>,
}
```

Cycle representation:

cycle_offsets[i] → start index into cycle_edges

Example:

cycle_offsets = [0,3,6]
cycle_edges   = [1,5,7,  2,9,11]

Cycle0 uses edges [1,5,7]
Cycle1 uses edges [2,9,11]

---

## Initialization Phase

1. Build graph of assets and tradable pairs.
2. Enumerate cycles up to length 3 or 4.
3. Store cycles in flat arrays.
4. Compute initial cycle_cost.
5. Populate edge_to_cycles reverse index.

Pseudo-code:

```rust
for cycle in cycles {
    let mut cost = 0.0;

    for edge in cycle {
        cost += edges[edge].weight;
        edge_to_cycles[edge].push(cycle_id);
    }

    cycle_cost.push(cost);
}
```

---

## Runtime Update Algorithm

When a price update occurs, only one edge weight changes.

Compute the delta and update affected cycles.

```rust
fn update_edge(graph: &mut Graph, edge: EdgeId, new_price: f64) {
    let new_weight = -new_price.ln();
    let old_weight = graph.edges[edge as usize].weight;

    let delta = new_weight - old_weight;

    graph.edges[edge as usize].weight = new_weight;

    for &cycle in &graph.edge_to_cycles[edge as usize] {
        let id = cycle as usize;

        graph.cycle_cost[id] += delta;

        if graph.cycle_cost[id] < 0.0 {
            let profit = (-graph.cycle_cost[id]).exp() - 1.0;

            emit_arbitrage(cycle, profit);
        }
    }
}
```

Hot path characteristics:

* O(cycles_per_edge)
* Only additions and comparisons
* No allocations

Typical cycles_per_edge:

10–80

---

## Performance Optimizations

Memory layout:

* Use contiguous vectors.
* Avoid pointer chasing.
* Preallocate everything.

SIMD opportunities:

Cycle updates are simple additions:

cycle_cost[i] += delta

This can be vectorized using AVX2/AVX512.

Branch prediction:

Check arbitrage condition last:

if cycle_cost < 0

Rare branch → good prediction.

Cache locality:

Store cycle_cost and edge_to_cycles contiguously.

---

## Optional Extensions

Order book depth:

Represent each level as separate edges:

BTC→USD@100000 size=1
BTC→USD@100100 size=2

Liquidity-aware cycles:

Attach volume capacity to edges.

Cross-venue transfers:

Add edges like:

BTC@Coinbase → BTC@Kraken

Fees:

Adjust weights:

weight = -ln(price * (1 - fee))

---

Performance Target

Per price update:

10–100 cycle updates

Expected latency:

200 ns – 2 µs on modern CPUs.

---

Key Principle

Never recompute the entire graph.

Only update cycles that contain the modified edge.

