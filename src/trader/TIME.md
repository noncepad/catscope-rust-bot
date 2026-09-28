# Time- Dimension DAG

Use this approach when exploiting non-atomic yield disparities over fixed holding periods (e.g., holding an LST for 3 days vs. standard SOL staking vs. borrowing SOL on Kamino/Marginfi to mint LSTs).

I want to include the direct redemption of liquid staking tokens as well.

#### 1. Graph Architecture

The graph is constructed as a **Time-Expanded Directed Acyclic Graph (DAG)** where each physical token is split into time-bound nodes.

#### 2. Structural Components

* **Nodes:** Defined as a tuple of `(Token, Time_t)`.
  * Example: `(SOL, t0)`, `(JitoSOL, t0)`, `(SOL, t1)`, `(JitoSOL, t1)`
* **Horizontal (Spot) Edges:** Connect tokens at the same timestamp `(Token_A, t)` $\to$ `(Token_B, t)`.
  * **Weight:** $w = -\ln(R_{\text{spot}})$
* **Vertical (Yield) Edges:** Connect an LST across time `(LST, t)` $\to$ `(LST, t + \Delta t)`.
  * **Weight:** $w = -\ln(1 + y \cdot \Delta t)$ where $y$ is the annualized yield rate (APY) and $\Delta t$ is the holding duration in years.
* **Borrowing / Financing Edges:** Connect a base asset across time `(SOL, t)` $\to$ `(SOL, t + \Delta t)`.
  * **Weight:** $w = -\ln(1 - r_{\text{borrow}} \cdot \Delta t)$ where $r_{\text{borrow}}$ is the lending protocol APY.

#### 3. Edge Weight Summary Table

| Edge Type | Connection | Weight Formula |
| :--- | :--- | :--- |
| **Spot Swap** | `(A, t)` $\to$ `(B, t)` | $-\ln(R_{\text{spot}}(A \to B))$ |
| **LST Accrual** | `(LST, t)` $\to$ `(LST, t + \Delta t)` | $-\ln(1 + y \cdot \Delta t)$ |
| **SOL Borrow** | `(SOL, t)` $\to$ `(SOL, t + \Delta t)` | $-\ln(1 - r_{\text{borrow}} \cdot \Delta t)$ |
| **Unstake Delay** | `(LST, t)` $\to$ `(SOL, t + T_{\text{unstake}})` | $-\ln(1 + y \cdot T_{\text{unstake}}) + \text{fee\_penalty}$ |

# path finding algorithm

### Calculating Arbitrage in Approach B (Time-Expanded Graph)

Unlike standard spot arbitrage, Approach B constructs a **Directed Acyclic Graph (DAG)**. Because time strictly increases ($t_0 \to t_1 \to t_2 \dots$), physical cycles are impossible, making standard Bellman-Ford cycle detection unnecessary.

Instead, arbitrage calculation reduces to a **Single-Source Shortest Path (SSSP) problem on a DAG** using Dynamic Programming.

---

### Algorithm: DAG Shortest Path via Topological Sort

Because the graph is a DAG, we can find the optimal arbitrage path in **$\mathcal{O}(V + E)$ time** instead of Bellman-Ford's $\mathcal{O}(V \cdot E)$.

#### 1. Pseudocode

```text
Algorithm FindTemporalArbitrage:
  Input: 
    - Graph G = (V, E)
    - Source Node: (SOL, t0)
    - Target Node: (SOL, t_N)
  Output: 
    - Arbitrage Path and Profit Ratio

  1. Initialize distances:
     FOR EACH node v in V:
         dist[v] = +infinity
         parent[v] = NULL
     dist[(SOL, t0)] = 0

  2. Generate Topological Order:
     Order all nodes primarily by timestamp t (t0, t1, ... t_N).
     Within the same timestamp t, order by dependency (e.g., input token before swap output).

  3. Relax Edges in Topological Order:
     FOR EACH node u in TopologicalOrder:
         IF dist[u] != +infinity:
             FOR EACH outgoing edge (u -> v) with weight w:
                 IF dist[u] + w < dist[v]:
                     dist[v] = dist[u] + w
                     parent[v] = u

  4. Calculate Profit at Target:
     final_weight = dist[(SOL, t_N)]
     profit_ratio = exp(-final_weight)

     IF profit_ratio > 1.0:
         path = ReconstructPath(parent, (SOL, t_N))
         RETURN path, profit_ratio
     ELSE:
         RETURN "No Arbitrage Opportunity", 0
