//! Structural factor extraction via the Normalized Graph Laplacian --
//! Phases 0-1 of `brain::multimodelv1::PLAN-1.md`: cluster tokens by real
//! liquidity coupling (not historical price correlation), expose the
//! resulting eigenvectors as structural factor loadings, and gate their
//! use with the two staleness triggers Phase 1 requires
//! ([`check_staleness`]).
//!
//! Pure/no host-import dependency, same testability discipline as
//! `pricegraph.rs`/`spfa.rs`/`credit.rs`/`derivative_router.rs` --
//! callers are responsible for turning real pool state (`DexState`, once
//! wired in a later phase) into [`LiquidityEdge`] values; this module
//! does no chain reads and knows nothing about `AccountId`s, mints, or
//! the wallet. Tokens are plain `usize` indices into whatever token list
//! the caller is tracking.
//!
//! Deliberately not wired into any bot mode yet -- see
//! `PLAN-1.md`'s Phase 0 note. `INSTRUCTIONS.md`'s own Go/`gonum`
//! snippet is a reference for the math only; this crate has no existing
//! linear-algebra dependency (checked `Cargo.toml` before writing this),
//! so eigendecomposition here is an in-house cyclic Jacobi solver rather
//! than a new dependency -- appropriate for the small, dense, symmetric
//! matrices this module deals with (a token-count-sized Laplacian, not a
//! sparse market-wide one), and this is explicitly the periodic
//! full-resync path (`PLAN-1.md` Phase 1), not a hot loop, so Jacobi's
//! O(n^3)-per-sweep cost is the intended trade-off, not an oversight.

/// One real liquidity pool between two tokens, already resolved by the
/// caller (real TVL/depth, not a placeholder) -- see the module doc
/// comment for why this module doesn't fetch this itself. `from`/`to`
/// are indices into the caller's own token list, `0..num_tokens` as
/// passed to [`FactorGraph::new`].
#[derive(Debug, Clone, Copy)]
pub struct LiquidityEdge {
    pub from: usize,
    pub to: usize,
    /// Effective liquidity depth for this pool, already in whatever real
    /// unit the caller chose (e.g. `ln(1 + TVL_usd)`, per
    /// `INSTRUCTIONS.md`'s own suggested weighting) -- this module treats
    /// it as an opaque non-negative edge weight, it does not itself log-
    /// or unit-transform raw TVL.
    pub liquidity: f64,
}

/// A token universe plus the real liquidity edges between them -- the
/// pure input to [`structural_factors`]. Symmetric by construction: an
/// edge is added once and counted in both tokens' degrees, matching
/// `INSTRUCTIONS.md`'s "arbitrage-free spot graphs allow execution in
/// both directions" reasoning (`A_ij = A_ji`).
pub struct FactorGraph {
    num_tokens: usize,
    edges: Vec<LiquidityEdge>,
}

impl FactorGraph {
    pub fn new(num_tokens: usize) -> Self {
        Self { num_tokens, edges: Vec::new() }
    }

    /// Adds a real edge. `liquidity <= 0.0` is silently dropped (an
    /// edge with zero or negative weight contributes nothing to the
    /// Laplacian and would only add noise) -- callers filtering by a
    /// minimum-liquidity threshold (`PLAN-1.md`'s later phases) should
    /// do so before calling this, not rely on this as the filter.
    pub fn add_edge(&mut self, from: usize, to: usize, liquidity: f64) {
        if liquidity > 0.0 {
            self.edges.push(LiquidityEdge { from, to, liquidity });
        }
    }

    fn degrees(&self) -> Vec<f64> {
        let mut degrees = vec![0.0; self.num_tokens];
        for e in &self.edges {
            degrees[e.from] += e.liquidity;
            degrees[e.to] += e.liquidity;
        }
        degrees
    }

    /// Builds the Symmetric Normalized Graph Laplacian,
    /// `L_sym = I - D^(-1/2) A D^(-1/2)` -- `INSTRUCTIONS.md` step 3.
    /// A token with zero total liquidity (isolated node, e.g. a curated
    /// symbol with no real pool yet) gets an all-zero row/column rather
    /// than a division-by-zero -- it contributes an eigenvalue of `0.0`
    /// with a trivial unit-vector eigenvector, which is the correct
    /// "no structural information about this token yet" answer, not an
    /// error.
    pub fn normalized_laplacian(&self) -> Vec<Vec<f64>> {
        let n = self.num_tokens;
        let degrees = self.degrees();
        let inv_sqrt_degree: Vec<f64> =
            degrees.iter().map(|&d| if d > 0.0 { 1.0 / d.sqrt() } else { 0.0 }).collect();

        let mut l = vec![vec![0.0; n]; n];
        for i in 0..n {
            if degrees[i] > 0.0 {
                l[i][i] = 1.0;
            }
        }
        for e in &self.edges {
            let off = -e.liquidity * inv_sqrt_degree[e.from] * inv_sqrt_degree[e.to];
            l[e.from][e.to] += off;
            l[e.to][e.from] += off;
        }
        l
    }
}

/// The real, reusable structural-factor result -- eigenvalues ascending
/// (`INSTRUCTIONS.md` step 4: `lambda_0 = 0` first, smallest non-zero
/// eigenvalues next), `eigenvectors[token][factor]` giving token `token`'s
/// loading on factor `factor` (columns, matching the reference Go
/// snippet's `eVecs.At(i, j)` convention).
///
/// Note on `lambda_0`'s eigenvector, corrected from `INSTRUCTIONS.md`'s
/// own simplified description ("the constant vector"): for the
/// *symmetric normalized* Laplacian specifically, the null-space
/// eigenvector is proportional to `sqrt(degree_i)`, not literally
/// constant, unless every token has equal total liquidity. (The
/// constant vector is the null vector of the *unnormalized* Laplacian
/// `D - A`; `L_sym`'s is `D^(1/2) * 1`, since
/// `L_sym (D^(1/2) 1) = D^(-1/2) L D^(-1/2) D^(1/2) 1 = D^(-1/2) L 1 = 0`.)
/// Getting this right matters for staleness/clustering checks built on
/// top of this (`PLAN-1.md` Phase 1) -- see this module's tests for a
/// worked example where token liquidity is unequal.
#[derive(Debug)]
pub struct StructuralFactors {
    pub eigenvalues: Vec<f64>,
    pub eigenvectors: Vec<Vec<f64>>,
}

/// Eigenvalues within this of `0.0` count as trivial (part of the
/// Laplacian's null eigenspace), not real structural signal --
/// live-observed (2026-09-07) this bot's real converged near-zero
/// eigenvalues sit at `1e-15`-ish float noise, and the smallest real
/// non-trivial eigenvalue this bot has ever seen live is nowhere close
/// to this either.
pub const TRIVIAL_EIGENVALUE_EPS: f64 = 1e-6;

/// How many of `eigenvalues`' smallest values are trivial (within
/// [`TRIVIAL_EIGENVALUE_EPS`] of `0.0`). Graph-theoretically this equals
/// the real number of connected components in the graph the eigenvalues
/// came from -- a disconnected graph Laplacian has exactly one zero
/// eigenvalue *per component*, each with its own indicator eigenvector
/// (zero everywhere outside that one component), not one eigenvalue
/// overall. `eigenvalues` must already be sorted ascending
/// (`structural_factors`'s own contract) -- this only scans a leading
/// prefix, not the whole slice.
pub fn trivial_eigenvalue_count(eigenvalues: &[f64]) -> usize {
    eigenvalues.iter().take_while(|&&ev| ev.abs() < TRIVIAL_EIGENVALUE_EPS).count()
}

/// Full eigendecomposition of `graph`'s normalized Laplacian --
/// `INSTRUCTIONS.md` step 4, the correctness baseline `PLAN-1.md`'s
/// Phase 0 asks for before any incremental rank-2 tracking is attempted.
pub fn structural_factors(graph: &FactorGraph) -> StructuralFactors {
    let l = graph.normalized_laplacian();
    let (eigenvalues, eigenvectors) = jacobi_eigen_symmetric(l);
    StructuralFactors { eigenvalues, eigenvectors }
}

impl StructuralFactors {
    /// Real, non-trivial eigenvector loadings only -- skips however many
    /// leading columns are trivial (see [`trivial_eigenvalue_count`]),
    /// not a hardcoded single column. A *connected* graph has exactly
    /// one trivial null eigenvector (eigenvalue `0.0`, column 0); this
    /// bot's real router-coverage-built graph is live-observed *not*
    /// always connected (2026-09-07: 60 of 147 curated tokens' own
    /// components on a real cycle), so the trivial eigenspace can be far
    /// wider than one column -- every extra trivial column is some
    /// *other* component's own indicator eigenvector (zero on every node
    /// outside its own tiny island), not real cross-token structural
    /// signal for tokens in a different component. Every real consumer
    /// of factor loadings (residuals, the Hawkes jump loop, the
    /// directional basket-builder) must go through this rather than
    /// slicing `eigenvectors` directly, so none of them can drift out of
    /// sync with each other on how many columns to skip.
    pub fn real_eigenvectors(&self) -> Vec<Vec<f64>> {
        let skip = trivial_eigenvalue_count(&self.eigenvalues);
        self.eigenvectors.iter().map(|row| row.get(skip..).unwrap_or(&[]).to_vec()).collect()
    }
}

// --- Phase 1 (`PLAN-1.md`): staleness detection --------------------------
//
// A full `structural_factors` resync (above) is always exactly
// orthonormal, to float precision -- Jacobi doesn't drift. The staleness
// gate below exists for the follow-up this repo doesn't have yet: an
// incremental rank-2-perturbation tracker that updates a maintained
// `V_k` per-swap instead of re-running the full O(n^3) decomposition
// every time (`PLAN-1.md`'s own "explicitly out of scope for v1" note).
// That kind of incremental update accumulates small floating-point error
// over many steps without periodic reorthogonalization, and nothing
// forces a resync just because time has passed if the event loop stalls.
// Both triggers below are independent and either alone is enough to
// distrust the factors -- see [`check_staleness`]'s doc comment for how
// a caller (Phase 4/5) is expected to use this.

/// Max age, in whole seconds, a full resync is trusted for before a
/// trade-decision path must refuse to open anything on these factors.
/// Same order of magnitude as `INSTRUCTIONS.md`'s own suggested
/// full-resync cadence (10-30s) -- treated here as an upper bound on
/// *trust*, not just a resync schedule: a resync that's overdue for any
/// reason (a stalled event loop, a burst of accounts backing up
/// processing) must stop new opens, not just eventually catch up.
pub const MAX_FACTOR_STALENESS_SECS: i64 = 30;

/// Max tolerated departure from exact orthonormality in a tracked
/// eigenvector matrix, `‖V^T V - I‖_F` (see [`orthogonality_drift`]).
/// Conservative by construction: real incremental-tracking drift
/// behavior isn't characterized yet, since nothing in this repo performs
/// the incremental update this threshold is meant to guard (that's the
/// deferred follow-up, not this phase) -- revisit this number once that
/// path exists and its real drift rate over a real trading session is
/// measured, don't treat it as final.
pub const MAX_ORTHOGONALITY_DRIFT: f64 = 1e-6;

/// `‖V^T V - I‖_F` -- the Frobenius norm of a `token x factor`
/// eigenvector matrix's departure from exact orthonormality. `0.0` for a
/// perfect orthonormal basis; grows as tracked columns drift away from
/// being unit-length and mutually perpendicular. A pure numerical
/// utility, independent of which Laplacian (if any) `eigenvectors` was
/// originally meant to represent -- it cannot tell you whether the
/// factors are *correct*, only whether they're still a valid orthonormal
/// basis, which is the cheap, necessary-but-not-sufficient check
/// incremental tracking needs between full resyncs.
pub fn orthogonality_drift(eigenvectors: &[Vec<f64>]) -> f64 {
    let n = eigenvectors.len();
    if n == 0 {
        return 0.0;
    }
    let k = eigenvectors[0].len();
    let mut sum_sq = 0.0;
    for i in 0..k {
        for j in 0..k {
            let dot: f64 = (0..n).map(|t| eigenvectors[t][i] * eigenvectors[t][j]).sum();
            let expected = if i == j { 1.0 } else { 0.0 };
            let diff = dot - expected;
            sum_sq += diff * diff;
        }
    }
    sum_sq.sqrt()
}

/// Phase 1's real gate -- both staleness triggers, independent of each
/// other. See [`check_staleness`]'s doc comment for how a caller should
/// use this.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StalenessCheck {
    /// `true` if too much wall-clock time has passed since the last full
    /// resync completed, or none ever has.
    pub time_stale: bool,
    /// `true` if the tracked eigenvector matrix has drifted too far from
    /// exact orthonormality to trust.
    pub drift_stale: bool,
}

impl StalenessCheck {
    pub fn is_stale(&self) -> bool {
        self.time_stale || self.drift_stale
    }
}

/// Evaluates both of Phase 1's staleness triggers against real,
/// caller-resolved inputs -- `now_secs`/`last_full_resync_secs` are real
/// UNIX-epoch seconds (same convention `leveragedloopv1`'s basis-trade
/// epoch gating already uses via `SystemTime`/`UNIX_EPOCH`, kept out of
/// this module itself per its own no-host-import discipline).
/// `eigenvectors` is whatever the caller currently trusts as its
/// structural factors -- fresh output of [`structural_factors`], or,
/// once it exists, an incrementally tracked `V_k`.
///
/// **Caller contract** (this module can't enforce it, only document it,
/// same as every other pure module in this crate): check
/// [`StalenessCheck::is_stale`] before *opening* anything new on these
/// factors -- never before *closing* an existing position. A stale-data
/// gate exists to stop new risk from being taken on bad information, not
/// to trap an existing position open while its own exit signal is
/// unavailable.
pub fn check_staleness(
    now_secs: i64,
    last_full_resync_secs: Option<i64>,
    eigenvectors: &[Vec<f64>],
) -> StalenessCheck {
    let time_stale = match last_full_resync_secs {
        None => true,
        Some(t) => now_secs.saturating_sub(t) > MAX_FACTOR_STALENESS_SECS,
    };
    let drift_stale = orthogonality_drift(eigenvectors) > MAX_ORTHOGONALITY_DRIFT;
    StalenessCheck { time_stale, drift_stale }
}

/// Classic cyclic Jacobi eigenvalue algorithm for a real symmetric
/// matrix -- sweeps every off-diagonal `(p, q)` pair, zeroing it with a
/// rotation, until the off-diagonal energy is negligible or
/// `MAX_SWEEPS` is hit. Returns eigenvalues ascending and the matching
/// eigenvector columns (`v[token][factor]`). Numerically robust and
/// simple to verify by hand for the small matrices this module deals
/// with -- see the module doc comment for why this isn't a borrowed
/// dependency.
fn jacobi_eigen_symmetric(mut a: Vec<Vec<f64>>) -> (Vec<f64>, Vec<Vec<f64>>) {
    const MAX_SWEEPS: usize = 100;
    const TOL: f64 = 1e-12;
    let n = a.len();
    let mut v = vec![vec![0.0; n]; n];
    for i in 0..n {
        v[i][i] = 1.0;
    }
    if n <= 1 {
        return (a.into_iter().enumerate().map(|(i, row)| row[i]).collect(), v);
    }

    for _ in 0..MAX_SWEEPS {
        let off_norm: f64 = (0..n)
            .flat_map(|p| (p + 1..n).map(move |q| (p, q)))
            .map(|(p, q)| a[p][q] * a[p][q])
            .sum();
        if off_norm < TOL {
            break;
        }
        for p in 0..n {
            for q in (p + 1)..n {
                if a[p][q].abs() < 1e-15 {
                    continue;
                }
                let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
                let t = if theta == 0.0 {
                    1.0
                } else {
                    theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt())
                };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;

                for k in 0..n {
                    if k != p && k != q {
                        let a_kp = a[k][p];
                        let a_kq = a[k][q];
                        a[k][p] = c * a_kp - s * a_kq;
                        a[p][k] = a[k][p];
                        a[k][q] = s * a_kp + c * a_kq;
                        a[q][k] = a[k][q];
                    }
                }
                let a_pp = a[p][p];
                let a_qq = a[q][q];
                let a_pq = a[p][q];
                a[p][p] = a_pp - t * a_pq;
                a[q][q] = a_qq + t * a_pq;
                a[p][q] = 0.0;
                a[q][p] = 0.0;

                for k in 0..n {
                    let v_kp = v[k][p];
                    let v_kq = v[k][q];
                    v[k][p] = c * v_kp - s * v_kq;
                    v[k][q] = s * v_kp + c * v_kq;
                }
            }
        }
    }

    let eigenvalues: Vec<f64> = (0..n).map(|i| a[i][i]).collect();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&i, &j| eigenvalues[i].total_cmp(&eigenvalues[j]));

    let sorted_eigenvalues = order.iter().map(|&i| eigenvalues[i]).collect();
    let sorted_eigenvectors: Vec<Vec<f64>> =
        (0..n).map(|row| order.iter().map(|&i| v[row][i]).collect()).collect();
    (sorted_eigenvalues, sorted_eigenvectors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn two_token_single_edge_has_known_spectrum() {
        // L_sym for a single edge is exactly [[1,-1],[-1,1]] regardless
        // of the edge's real weight (the weight cancels in the
        // normalization) -- a hand-checkable identity: eigenvalues 0
        // and 2, eigenvectors [1,1]/sqrt(2) and [1,-1]/sqrt(2).
        let mut g = FactorGraph::new(2);
        g.add_edge(0, 1, 42_000_000.0);
        let f = structural_factors(&g);
        assert!(approx(f.eigenvalues[0], 0.0));
        assert!(approx(f.eigenvalues[1], 2.0));
        // lambda_0's eigenvector (column 0 across both tokens): equal-
        // degree tokens, so sqrt(degree) reduces to the constant
        // direction -- both components equal magnitude, same sign.
        let factor0: Vec<f64> = (0..2).map(|t| f.eigenvectors[t][0]).collect();
        let factor1: Vec<f64> = (0..2).map(|t| f.eigenvectors[t][1]).collect();
        assert!(approx(factor0[0].abs(), factor0[1].abs()));
        assert!(approx(factor0[0] * factor0[1], factor0[0].abs() * factor0[1].abs())); // same sign
        // lambda_2's eigenvector: opposite sign, equal magnitude.
        assert!(approx(factor1[0].abs(), factor1[1].abs()));
        assert!(factor1[0] * factor1[1] < 0.0);
    }

    #[test]
    fn complete_three_token_graph_matches_known_kn_spectrum() {
        // Normalized-Laplacian spectrum of the complete graph K_n
        // (equal edge weights) is a known closed form: 0 (once), and
        // n/(n-1) (with multiplicity n-1). For K_3: 0, 1.5, 1.5.
        let mut g = FactorGraph::new(3);
        g.add_edge(0, 1, 1.0);
        g.add_edge(1, 2, 1.0);
        g.add_edge(0, 2, 1.0);
        let f = structural_factors(&g);
        assert!(approx(f.eigenvalues[0], 0.0));
        assert!(approx(f.eigenvalues[1], 1.5));
        assert!(approx(f.eigenvalues[2], 1.5));
    }

    #[test]
    fn isolated_token_is_a_trivial_zero_eigenvalue() {
        // Token 2 has no edges at all -- must not divide by zero, and
        // must contribute its own independent lambda=0 eigenvalue
        // (there are two zero eigenvalues here: one for the connected
        // {0,1} component, one for isolated token 2).
        let mut g = FactorGraph::new(3);
        g.add_edge(0, 1, 10.0);
        let f = structural_factors(&g);
        let zero_count = f.eigenvalues.iter().filter(|&&l| approx(l, 0.0)).count();
        assert_eq!(zero_count, 2);
    }

    #[test]
    fn trivial_eigenvalue_count_matches_real_component_count() {
        // Same fixture as `isolated_token_is_a_trivial_zero_eigenvalue`:
        // two components ({0,1} and isolated token 2) means two trivial
        // eigenvalues, not one -- this is the exact live bug this fn
        // exists to catch (2026-09-07: 60-component real graph, not the
        // single-component case `.get(1..)` used to assume everywhere).
        let mut g = FactorGraph::new(3);
        g.add_edge(0, 1, 10.0);
        let f = structural_factors(&g);
        assert_eq!(trivial_eigenvalue_count(&f.eigenvalues), 2);
    }

    #[test]
    fn trivial_eigenvalue_count_is_one_for_a_single_connected_component() {
        let mut g = FactorGraph::new(3);
        g.add_edge(0, 1, 1.0);
        g.add_edge(1, 2, 1.0);
        let f = structural_factors(&g);
        assert_eq!(trivial_eigenvalue_count(&f.eigenvalues), 1);
    }

    #[test]
    fn real_eigenvectors_skips_every_trivial_column_not_just_the_first() {
        // Two components -> two trivial columns must be skipped, leaving
        // exactly one real column per token for this 3-token graph.
        let mut g = FactorGraph::new(3);
        g.add_edge(0, 1, 10.0);
        let f = structural_factors(&g);
        let real = f.real_eigenvectors();
        assert_eq!(real.len(), 3);
        for row in &real {
            assert_eq!(row.len(), 1);
        }
    }

    #[test]
    fn real_eigenvectors_skips_only_column_zero_for_a_single_component() {
        let mut g = FactorGraph::new(2);
        g.add_edge(0, 1, 1.0);
        let f = structural_factors(&g);
        let real = f.real_eigenvectors();
        assert_eq!(real[0].len(), 1);
        assert!(approx(real[0][0], f.eigenvectors[0][1]));
    }

    #[test]
    fn unequal_liquidity_star_graph_null_eigenvector_matches_sqrt_degree() {
        // INSTRUCTIONS.md's own 4-token example (SOL hub, unequal edge
        // weights) -- rather than hand-computing the full spectrum,
        // verify the mathematical invariant that actually matters for
        // later phases: lambda_0's eigenvector is proportional to
        // sqrt(degree_i), not the plain constant vector, whenever
        // degrees differ (see structural_factors' doc comment). SOL (0)
        // is the hub; its degree is far larger than JUP/BONK's.
        let mut g = FactorGraph::new(4); // 0=SOL, 1=mSOL, 2=JUP, 3=BONK
        g.add_edge(0, 1, 50_000_000.0);
        g.add_edge(0, 2, 10_000_000.0);
        g.add_edge(0, 3, 2_000_000.0);
        g.add_edge(2, 3, 500_000.0);
        let f = structural_factors(&g);
        assert!(approx(f.eigenvalues[0], 0.0));

        let degrees = g.degrees();
        // Column 0 of `eigenvectors` (loading on factor 0, across every
        // token) is lambda_0's eigenvector -- not row 0, which would be
        // token 0's loadings across every factor instead.
        let v0: Vec<f64> = (0..4).map(|t| f.eigenvectors[t][0]).collect();
        // v0[i] / sqrt(degrees[i]) should be the same real constant for
        // every token (up to the whole eigenvector's overall sign).
        let ratios: Vec<f64> = (0..4).map(|i| v0[i] / degrees[i].sqrt()).collect();
        for r in &ratios[1..] {
            assert!(approx(*r, ratios[0]));
        }
        // And that ratio must be nonzero -- otherwise this test would
        // trivially pass on an all-zero eigenvector.
        assert!(ratios[0].abs() > 1e-6);
    }

    #[test]
    fn eigenvectors_are_orthonormal() {
        let mut g = FactorGraph::new(4);
        g.add_edge(0, 1, 50_000_000.0);
        g.add_edge(0, 2, 10_000_000.0);
        g.add_edge(0, 3, 2_000_000.0);
        g.add_edge(2, 3, 500_000.0);
        let f = structural_factors(&g);
        let n = f.eigenvalues.len();
        for i in 0..n {
            for j in 0..n {
                let dot: f64 = (0..n).map(|k| f.eigenvectors[k][i] * f.eigenvectors[k][j]).sum();
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!((dot - expected).abs() < 1e-8, "V^T V not identity at ({i},{j}): {dot}");
            }
        }
    }

    #[test]
    fn eigenvalues_are_bounded_zero_to_two() {
        // A well-known spectral property of the normalized Laplacian:
        // every eigenvalue lies in [0, 2]. A useful sanity check that
        // doesn't depend on hand-computing the exact spectrum.
        let mut g = FactorGraph::new(5);
        g.add_edge(0, 1, 3.0);
        g.add_edge(1, 2, 7.0);
        g.add_edge(2, 3, 1.5);
        g.add_edge(3, 4, 9.0);
        g.add_edge(0, 4, 2.2);
        g.add_edge(1, 4, 0.5);
        let f = structural_factors(&g);
        for &l in &f.eigenvalues {
            assert!((-1e-9..=2.0 + 1e-9).contains(&l), "eigenvalue out of bounds: {l}");
        }
    }

    #[test]
    fn add_edge_drops_non_positive_liquidity() {
        let mut g = FactorGraph::new(2);
        g.add_edge(0, 1, 0.0);
        g.add_edge(0, 1, -5.0);
        // No real edge was added -- both tokens are isolated, so the
        // Laplacian is all-zero and both eigenvalues are exactly 0.
        let f = structural_factors(&g);
        assert!(approx(f.eigenvalues[0], 0.0));
        assert!(approx(f.eigenvalues[1], 0.0));
    }

    // --- Phase 1: staleness detection -----------------------------------

    fn star_graph_factors() -> StructuralFactors {
        let mut g = FactorGraph::new(4);
        g.add_edge(0, 1, 50_000_000.0);
        g.add_edge(0, 2, 10_000_000.0);
        g.add_edge(0, 3, 2_000_000.0);
        g.add_edge(2, 3, 500_000.0);
        structural_factors(&g)
    }

    #[test]
    fn orthogonality_drift_is_near_zero_for_a_fresh_full_resync() {
        // A real structural_factors() output is orthonormal to float
        // precision, well inside the threshold meant for accumulated
        // incremental-tracking drift -- this is the headroom real usage
        // relies on: a fresh resync must never itself trip the gate.
        let f = star_graph_factors();
        let drift = orthogonality_drift(&f.eigenvectors);
        assert!(drift < MAX_ORTHOGONALITY_DRIFT, "fresh resync drift too high: {drift}");
    }

    #[test]
    fn orthogonality_drift_detects_a_corrupted_matrix() {
        // Simulates what accumulated incremental-tracking error would
        // look like: take a real orthonormal basis and perturb one
        // component -- no longer unit-length/orthogonal, and the drift
        // metric must catch it well above the threshold.
        let f = star_graph_factors();
        let mut corrupted = f.eigenvectors;
        corrupted[0][0] += 0.1;
        let drift = orthogonality_drift(&corrupted);
        assert!(drift > MAX_ORTHOGONALITY_DRIFT, "corrupted matrix not flagged: {drift}");
    }

    #[test]
    fn orthogonality_drift_empty_matrix_is_zero() {
        let empty: Vec<Vec<f64>> = Vec::new();
        assert_eq!(orthogonality_drift(&empty), 0.0);
    }

    #[test]
    fn check_staleness_never_resynced_is_always_time_stale() {
        let f = star_graph_factors();
        let check = check_staleness(1_000, None, &f.eigenvectors);
        assert!(check.time_stale);
        assert!(!check.drift_stale);
        assert!(check.is_stale());
    }

    #[test]
    fn check_staleness_within_bound_is_not_time_stale() {
        let f = star_graph_factors();
        let now = 1_000;
        let last_resync = now - MAX_FACTOR_STALENESS_SECS; // exactly at the bound
        let check = check_staleness(now, Some(last_resync), &f.eigenvectors);
        assert!(!check.time_stale, "exact boundary must not be stale (strict >, not >=)");
        assert!(!check.is_stale());
    }

    #[test]
    fn check_staleness_past_bound_is_time_stale() {
        let f = star_graph_factors();
        let now = 1_000;
        let last_resync = now - MAX_FACTOR_STALENESS_SECS - 1;
        let check = check_staleness(now, Some(last_resync), &f.eigenvectors);
        assert!(check.time_stale);
        assert!(check.is_stale());
    }

    #[test]
    fn check_staleness_drift_alone_marks_stale_even_with_fresh_resync() {
        let f = star_graph_factors();
        let mut corrupted = f.eigenvectors;
        corrupted[0][0] += 0.1;
        let now = 1_000;
        // Resync just completed (not time-stale at all) but the matrix
        // handed in is corrupted -- drift alone must be enough.
        let check = check_staleness(now, Some(now), &corrupted);
        assert!(!check.time_stale);
        assert!(check.drift_stale);
        assert!(check.is_stale());
    }

    #[test]
    fn check_staleness_good_data_and_recent_resync_is_not_stale() {
        // The real end-to-end Phase 0 + Phase 1 path: a real resync's
        // real output, checked immediately, must never be stale.
        let f = star_graph_factors();
        let now = 1_000;
        let check = check_staleness(now, Some(now), &f.eigenvectors);
        assert!(!check.is_stale());
    }
}
