  The core framing is sound but has one real gap and several things that need to be said out loud before this becomes a bot mode.

  1. It's an equity quant playbook wearing a Solana costume, and the costume matters more than the doc admits.
  Everything in section 1-2 (PCA/eigen factor models, Ledoit-Wolf shrinkage, OU mean-reversion) is textbook cross-sectional equity stat-arb. That's fine as math, but the doc treats "adapt to the graph" as a one-time transform (use the Laplacian instead of raw covariance)
  when the actual hard problem is that your graph is the price discovery mechanism, not a side-channel on it. In equities, the covariance matrix and the tradable price are separate things. Here, the edge weight you're building the Laplacian from (liquidity depth) is
  also exactly what determines your execution cost when you act on the signal. A cluster that looks "tightly coupled" by TVL can still be catastrophically expensive to actually trade in size — the model can be right about structure and still lose money entering the
  trade. The doc's "Practical Thresholds" section gestures at this (drop <$500 edges) but that's a floor, not a fix; nothing here sizes positions against real slippage/depth the way, say, pricegraph.rs's arb search already has to.

  2. The stat-arb section names the real risk and then doesn't resolve it.
  Section 3B is honest that borrow APY can outrun mean-reversion, but the "solution" is "ensure your horizon is shorter than the rate bleed" — that's not a solution, it's a restatement of the risk with no mechanism to enforce it. You already have this exact problem
  solved elsewhere in this codebase: [[feedback_derivative_router]] / the basis-trade work you just ran live explicitly gates on borrow_apy vs funding_pct before opening. This plan needs the equivalent gate before any residual-driven short, not after.

  3. Section 4 (incremental eigen-updates) is the part most likely to be wrong in practice, and it's the part with the least detail.
  Rank-2 perturbation / Rayleigh-Ritz tracking of the top-k eigenvectors is a real technique, but it's numerically delicate — subspace tracking accumulates drift and can silently diverge from the true eigenbasis between full resyncs, especially with the kind of
  discontinuous liquidity events Solana pools actually have (a pool getting drained or a new pool appearing isn't a small perturbation, it's closer to a rank change in the effective graph, not a rank-2 edge tweak). A 10-30s full resync as the backstop is a reasonable
  mitigation, but the doc doesn't specify how you'd detect that the tracked factors have drifted enough to distrust them in between resyncs — you could be trading off stale factor loadings for up to 30s with no signal that they're stale.

  4. No execution/slippage model, no position sizing, no unwind path.
  Every other bot mode in this repo (leveragedloopv1, perpfundingv1, testperpv1) was built deposit-first-close-tested, with explicit real-money sizing constants and a proven deleverage path before the open path ever ran for real. This doc is pure signal generation —
  "send order" is the last box in the section-4 diagram with nothing behind it. Before this is a bot mode rather than a research notebook, it needs the same treatment: how much capital per idiosyncratic-alpha bet, what closes a stat-arb leg if the residual keeps widening
  instead of reverting, what happens to open dispersion-trade legs if the bot restarts mid-cycle (every other mode here deliberately re-derives position state from on-chain reads rather than trusting bot-local state — this plan doesn't mention where its factor
  loadings/residual history would live or survive a restart).

  5. The Go snippet is a reference sketch, not usable as-is — fmt.println on line 216 is lowercase and won't compile, and mat.EigenSym on a dense N×N matrix is fine for a demo but is exactly the O(N³) cost section 4 says is too slow for real-time use; the snippet and the
  "don't do this in the hot path" advice are in tension with each other by design (it's meant as the periodic full-resync, not the hot path), but that's not stated.

  Bottom line: the graph-Laplacian framing is a legitimately good fit for the arb-free graph you already have, and the factor-clustering idea (group tokens by real liquidity coupling, not spurious price correlation) is more honest than a naive equity-style covariance
  approach would be. But this is a signal-generation research doc, not an execution plan — it's missing the borrow-cost gating, slippage-aware sizing, staleness detection, and restart-safe state design that every other real trading mode in this repo already had to earn
  the hard way before going live.
