## eigen vector multi factor model

**Yes, an eigenvalue-based multi-factor model (such as Principal Component Analysis or Truncated SVD) is an excellent mathematical framework for your goals**, but because you are trading Solana spot assets using a sparse arbitrage-free pricing graph, you must adapt standard equity-style factor models to respect the underlying graph structure.

Below is an engineering and quantitative breakdown of why this approach works, how to properly adapt it to your graph, and how to use it specifically for directional, correlation, and volatility trades.

### 1. Why an Eigenvalue Multi-Factor Model Fits Your Setup

By performing an Eigenvalue Decomposition (or Singular Value Decomposition) on the covariance matrix derived from your arbitrage-free pricing graph, you transform a high-dimensional, noisy token market into orthogonal factors:

$$\mathbf{R} = \mathbf{F} \mathbf{L}^T + \mathbf{\epsilon}$$

* **Eigenvector 1 (Market/Beta Factor):** Captures general SOL and overall crypto market directional moves.
* **Eigenvectors 2 & 3 (Sector/L1/DeFi Factors):** Capture co-movements between token clusters (e.g., Solana Memes, Liquid Staking Tokens, AI tokens).
* **Residuals ($\mathbf{\epsilon}$):** Capture idiosyncratic token noise—the exact signal needed for relative value, statistical arbitrage, and correlation trades.

### 2. Adapting the Model to Your Sparse Graph

A common mistake is constructing a sample covariance matrix from raw price series without taking advantage of your sparse arbitrage-free graph. Instead, combine the graph topology with your factor model:

#### A. Construct Graph Laplacians for Structural Factors

Your sparse graph provides explicit information about liquidity pools, edge weights (pool depth), and direct implied pricing. You can run spectral decomposition directly on the **Normalized Graph Laplacian** ($\mathbf{L} = \mathbf{I} - \mathbf{D}^{-1/2}\mathbf{A}\mathbf{D}^{-1/2}$):

* The smallest non-zero eigenvalues and corresponding eigenvectors identify **tightly coupled liquidity clusters**.
* Tokens with strong edge weights (deep direct pools) will naturally be grouped together, preventing false statistical correlation between two illiquid tokens that happen to drift together due to low volume.

#### B. Shrinkage and Regularization

Solana spot markets suffer from high tail-risk, sudden volume shifts, and dynamic pool creations. Standard sample covariance matrices become ill-conditioned quickly. Use **Ledoit-Wolf shrinkage** or **Random Matrix Theory (Marchenko-Pastur filtering)** on the covariance matrix before running Eigendecomposition to separate signal from statistical noise.

### 3. Executing Your Specific Trade Types

#### A. Directional & Multi-Factor Neutral Bets

* **Implementation:** Express a directional view on a specific token while hedging out systemic market risk.
* **How to trade:** If you are bullish on a specific Solana token $A$, project its returns against the top $k$ principal components (factors). Sell a basket of the factor portfolio proportional to $A$'s factor loadings ($\beta_k$). You now hold a pure **idiosyncratic alpha position** decoupled from broader Solana market crashes.

#### B. Correlation & Pair/Basket Trades (Stat-Arb)

* **Implementation:** Trade the residual return mean-reversion.
* **How to trade:** Calculate the residual vector $\mathbf{\epsilon}_t = \mathbf{r}_t - \sum \beta_k \mathbf{F}_k$. The residuals should be stationary (Ornstein-Uhlenbeck process). When a token’s residual deviates beyond 2–3 standard deviations from its rolling mean relative to its factor group:
* **Long** the underperforming token.
* **Short** the factor basket (or overperforming peer token within the same eigenvector cluster).
* **Solana Spot Note:** Since shorting spot on Solana requires borrowing via margin protocols (e.g., Kamino, Marginfi, Drift), ensure your mean-reversion time horizon is shorter than the borrow interest rate bleed.

#### C. Volatility Trades (Dispersion Trading)

* **Implementation:** Trade index/factor volatility against constituent token volatility.
* **How to trade:** Because direct options markets on Solana spot tokens (outside SOL itself) are often illiquid, you can construct **synthetic volatility trades (dispersion)** using dynamic rebalancing or delta-hedging strategies:
* **Short Factor Volatility / Long Constituent Volatility:** If correlation across Solana tokens drops, individual tokens move independently. Long a basket of high-idiosyncratic-volatility spot tokens, dynamically rebalancing back to factor-neutrality as prices move, while shorting the market index (SOL futures or perpetuals on Drift/Hyperliquid) to capture the variance risk premium gap.

### Key Operational Challenges on Solana

1. **Borrow Rate Asymmetry:** Shorting spot tokens requires borrowing. If a token's borrow APR spikes to 50%+ during a squeeze, it can destroy the statistical advantage of your factor-neutral pair trade before the residual mean-reverts.
2. **Liquidity Sinks & MEV:** When rebalancing multi-leg factor portfolios across Solana DEXs, ensure your arbitrage-free pricing graph incorporates real-time swap fees and priority fees/Jito tips to prevent execution slippage from eroding factor alpha.

## graph laplacians against sparse graph

Constructing a Graph Laplacian from your sparse arbitrage-free pricing graph allows you to extract **structural market factors** directly from network topology and liquidity depth rather than relying purely on historical price time series.

Here is the mathematical framework and a complete **Go** implementation using sparse matrix principles to construct and decompose the Normalized Graph Laplacian.

### Step 1: Formulate the Weighted Adjacency Matrix ($A$)

In a Solana spot pricing graph, nodes represent **tokens** ($N$) and edges represent **direct liquidity pools** (e.g., Raydium, Orca, Meteora).

1. **Define Edge Weights ($W_{ij}$):** Do not use simple binary connectivity. Set $W_{ij}$ proportional to the **effective liquidity depth** or swap capacity between Token $i$ and Token $j$:

$$W_{ij} = \log\left(1 + \text{TVL}_{ij}\right) \quad \text{or} \quad W_{ij} = \frac{\text{Liquidity}_{ij}}{\text{Slippage Tolerance}}$$

2. **Ensure Symmetry:** Because arbitrage-free spot graphs allow execution in both directions, set $A_{ij} = A_{ji} = W_{ij}$.

### Step 2: Compute the Degree Matrix ($D$)

The Degree Matrix $D$ is a diagonal matrix where each diagonal entry $D_{ii}$ represents the total liquidity connected to Token $i$:

$$D_{ii} = \sum_{j=1}^N A_{ij}$$

### Step 3: Compute the Normalized Graph Laplacian ($L_{sym}$)

While the unnormalized Laplacian is $L = D - A$, the **Symmetric Normalized Graph Laplacian** ($L_{sym}$) is essential for financial assets because token liquidity spans multiple orders of magnitude (e.g., SOL/USDC vs. a newly launched meme pair).

$$L_{sym} = D^{-1/2} L D^{-1/2} = I - D^{-1/2} A D^{-1/2}$$

Entry-wise, each element is calculated as:

$$(L_{sym})_{ij} = \begin{cases}  1 & \text{if } i = j \text{ and } D_{ii} \neq 0 \\ -\frac{A_{ij}}{\sqrt{D_{ii} D_{jj}}} & \text{if } i \neq j \text{ and } (i,j) \text{ is an edge} \\ 0 & \text{otherwise} \end{cases}$$

### Step 4: Extract Structural Factors via Eigendecomposition

Solve the eigenvalue problem for $L_{sym}$:

$$L_{sym} \mathbf{v}_k = \lambda_k \mathbf{v}_k$$

* **$\lambda_0 = 0$ (Eigenvector $\mathbf{v}_0$):** The constant vector representing the overall connected market component.
* **Smallest non-zero eigenvalues ($\lambda_1, \lambda_2, \dots, \lambda_k$):** The corresponding eigenvectors ($\mathbf{v}_1, \mathbf{v}_2, \dots$) define the **lowest-energy graph cuts**.
* Tokens with similar sign/magnitude in $\mathbf{v}_1$ belong to the same tightly coupled liquidity cluster.
* These eigenvectors serve as your **structural factor loadings** ($\beta$), grouping assets by structural market flow rather than statistical noise.

### Go Implementation

Below is a complete Go snippet using `gonum/mat` to construct $L_{sym}$ from a sparse edge list and solve for the structural factors.

```go
package main

import (
 "fmt"
 "math"

 "gonum.org/v1/gonum/mat"
)

// Edge represents a liquidity pool between two tokens in your arbitrage graph
type Edge struct {
 FromToken int     // Token index i
 ToToken   int     // Token index j
 Liquidity float64 // TVL or depth in USD
}

type GraphLaplacian struct {
 NumTokens int
 Edges     []Edge
}

// ComputeNormalizedLaplacian builds L_sym = I - D^(-1/2) * A * D^(-1/2)
func (g *GraphLaplacian) ComputeNormalizedLaplacian() *mat.SymDense {
 n := g.NumTokens

 // 1. Calculate Degree Vector D_ii
 degrees := make([]float64, n)
 for _, e := range g.Edges {
  degrees[e.FromToken] += e.Liquidity
  degrees[e.ToToken] += e.Liquidity
 }

 // 2. Precompute Inverse Square Root Degree: D^(-1/2)
 invSqrtD := make([]float64, n)
 for i := 0; i < n; i++ {
  if degrees[i] > 0 {
   invSqrtD[i] = 1.0 / math.Sqrt(degrees[i])
  } else {
   invSqrtD[i] = 0.0
  }
 }

 // 3. Construct Dense L_sym (using SymDense for symmetric efficiency)
 lSymData := make([]float64, n*n)
 lSym := mat.NewSymDense(n, lSymData)

 // Set Identity Diagonal (1.0 for non-isolated nodes)
 for i := 0; i < n; i++ {
  if degrees[i] > 0 {
   lSym.SetSym(i, i, 1.0)
  }
 }

 // Set Off-Diagonal Values: - A_ij / sqrt(D_ii * D_jj)
 for _, e := range g.Edges {
  i, j := e.FromToken, e.ToToken
  val := -e.Liquidity * invSqrtD[i] * invSqrtD[j]
  lSym.SetSym(i, j, val)
 }

 return lSym
}

// ExtractStructuralFactors calculates the Eigenvalues & Eigenvectors of L_sym
func ExtractStructuralFactors(lSym *mat.SymDense) ([]float64, *mat.Dense) {
 var eig mat.EigenSym
 
 // Perform symmetric eigendecomposition
 ok := eig.Factorize(lSym, true)
 if !ok {
  panic("Eigendecomposition failed to converge")
 }

 // Extract Eigenvalues (sorted in ascending order by gonum)
 lambdas := eig.Values(nil)

 // Extract Eigenvectors (Columns represent structural factor vectors)
 var eVecs mat.Dense
 eig.VectorsTo(&eVecs)

 return lambdas, &eVecs
}

func main() {
 // Example: 4 Solana tokens (e.g., 0: SOL, 1: mSOL, 2: JUP, 3: BONK)
 numTokens := 4
 edges := []Edge{
  {FromToken: 0, ToToken: 1, Liquidity: 50_000_000.0}, // SOL <-> mSOL (Very Deep)
  {FromToken: 0, ToToken: 2, Liquidity: 10_000_000.0}, // SOL <-> JUP
  {FromToken: 0, ToToken: 3, Liquidity: 2_000_000.0},  // SOL <-> BONK
  {FromToken: 2, ToToken: 3, Liquidity: 500_000.0},    // JUP <-> BONK
 }

 graph := GraphLaplacian{NumTokens: numTokens, Edges: edges}
 lSym := graph.ComputeNormalizedLaplacian()

 lambdas, eVecs := ExtractStructuralFactors(lSym)

 fmt.Println("--- Eigenvalues (Graph Energies) ---")
 for i, l := range lambdas {
  fmt.Printf("Factor %d (λ_%d): %.4f\n", i, i, l)
 }

 fmt.Println("\n--- Structural Factor Loadings (Eigenvector Matrix) ---")
 r, c := eVecs.Dims()
 for i := 0; i < r; i++ {
  fmt.Printf("Token %d Loadings: ", i)
  for j := 0; j < c; j++ {
   fmt.Printf("%8.4f ", eVecs.At(i, j))
  }
  fmt.println()
 }
}

```

### Step 5: How to Use These Loadings in Your Factor Model

1. **Construct the Structural Factor Matrix ($\mathbf{V}_k$):** Select the eigenvectors corresponding to the smallest $k$ non-zero eigenvalues ($\lambda_1 \dots \lambda_k$).
2. **Project Asset Returns:** When evaluating returns $\mathbf{R}_t$, project them onto your graph factors:

$$\mathbf{F}_t = \mathbf{V}_k^T \mathbf{R}_t$$

3. **Calculate Residuals for Mean Reversion:**

$$\mathbf{\epsilon}_t = \mathbf{R}_t - \mathbf{V}_k \mathbf{F}_t$$

Any asset where $\mathbf{\epsilon}_t$ deviates significantly from zero is mispriced relative to its structural liquidity cluster in your arbitrage graph.

## non-uniform time step

When constructing your graph from asynchronous, non-uniform updates (such as continuous Solana DEX swaps or liquidity pool events), static graph algorithms fail because **edges arrive at irregular timestamps $t$ and decay at different rates**.

To solve this, you must treat your graph as a **Continuous-Time Dynamic Graph (CTDG)**. This requires transforming discrete pool updates into a **smooth, continuously decaying Graph Laplacian**, and updating your factor loadings incrementally without recalculating full eigendecompositions.

### Step 1: Exponential Time-Decay for Edge Weights

Instead of setting $A_{ij} = \text{Liquidity}_{ij}$, weight each edge dynamically based on **how recently it was updated**. This prevents stale pools from skewing current price relationships while smoothing asynchronous updates.

For an edge update between Token $i$ and Token $j$ occurring at time $t_{event}$:

$$W_{ij}(t) = \text{Liquidity}_{ij} \cdot e^{-\lambda (t - t_{event})}$$

* **$\lambda$ (Decay Rate):** Controls memory length. For high-frequency Solana markets, set $\lambda = \frac{\ln(2)}{\tau}$, where $\tau$ is your target half-life (e.g., $\tau = 30 \text{ seconds}$).
* **Real-time Evaluation:** When a new update arrives at time $t_{now}$, you only update the specific entry $(i, j)$ in $A_{ij}(t_{now})$ and multiply the rest of the graph's weight matrix by $e^{-\lambda \Delta t}$.

### Step 2: Continuous Implied Returns via Point Processes

Because price observations arrive non-uniformly, calculating continuous returns $R_i(t) = \frac{P_i(t) - P_i(t - \Delta t)}{P_i(t - \Delta t)}$ creates asynchronous time gaps.

Convert your discrete price updates into a **Continuous-Time Exponentially Weighted Moving Average (EMA) Price State**:

```
Price State:      P_i(t) = P_i(last)   (Forward-fill last known execution price)
Price Variance:   σ²_i(t) = Exponentially decayed return variance

```

Whenever a swap event hits pool $(i, j)$ at time $t$:

1. Calculate implied instantaneous return $\Delta r_{ij}(t) = \ln(P_{implied}) - \ln(P_{last})$.
2. Update the joint covariance term between Token $i$ and Token $j$ incrementally using an **Online EMA Covariance update**:

$$\Sigma_{ij}(t) = (1 - \alpha) \Sigma_{ij}(t_{prev}) + \alpha \cdot (\Delta r_i \cdot \Delta r_j)$$

where $\alpha = 1 - e^{-\frac{\Delta t}{\tau}}$.

### Step 3: Low-Rank Perturbation (Fast Factor Updates)

Recomputing the full Eigendecomposition ($O(N^3)$) of $L_{sym}$ on every single block update is too slow for real-time trading.

Because an asynchronous pool update only alters **two rows and two columns** of the Graph Matrix $L_{sym}$, it acts as a **Rank-2 Perturbation**:

$$L_{sym}(t_{new}) = L_{sym}(t_{old}) + \mathbf{U} \mathbf{V}^T$$

You can update your factor loadings ($\mathbf{v}_k$) in **$O(N \cdot k^2)$ time** using **Rayleigh-Ritz Truncated Eigendecomposition Tracking**:

1. **Keep the top $k$ Eigenvectors:** Store the $N \times k$ matrix $V_k$ representing your major structural factors.
2. **Project the Update:** When a single edge $(i, j)$ changes by $\Delta W_{ij}$, project the sparse change vector onto the subspace of your current factors $V_k$.
3. **Rotate Eigenvectors:** Compute the small $k \times k$ eigensystem of the projected update and rotate $V_k$.

This allows you to maintain continuous structural factors in sub-millisecond execution loops in Go or Rust.

### Step 4: The Continuous-Time Structural Factor Architecture

The end-to-end event loop operates as follows:

```
[WebSocket Pool Updates] ──> [Asynchronous Arrival]
                                  │
                                  ▼
                     [Update Sparse Edge W_ij(t)]
                     [Decay Inactive Edges via Exp(-λ Δt)]
                                  │
                                  ▼
                   [Recompute Local Degrees D_ii, D_jj]
                                  │
                                  ▼
             [Rank-2 Rank-Update on Eigenvectors V_k]
                                  │
                                  ▼
            [Residual Mispricing: ε_i = R_i - V_k * F_k] ──> [Send Order]

```

### Practical Thresholds for Solana

* **Update Frequency:** Recompute the local edge update $W_{ij}(t)$ on **every block/slot (~400ms)** or every transaction.
* **Full Eigendecomposition Resync:** Run the full $O(N^3)$ or Krylov-subspace Eigendecomposition in a background thread once every **10–30 seconds** to reset numerical drift from the incremental rank-updates.
* **Filter Minimum Liquidity:** Drop edges where $W_{ij}(t) < \epsilon$ (e.g., liquidity $< \$500$) from the sparse matrix calculations. This keeps $L_{sym}$ extremely sparse and speeds up factor tracking.
