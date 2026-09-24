//! Simplified single-position margin/liquidation-risk math.
//!
//! **Documented simplification, same class as this session's other
//! flat-fee/constant-product approximations** (e.g. Pump.fun/PumpSwap's
//! flat fee estimate, CLMM-as-constant-product): the real protocol margin
//! engine (`rust/math/src/margin_calc.rs` in `rise-public`, 707 lines)
//! computes cross-margin health across a trader's *entire* position
//! portfolio, with per-tier leverage scaling, oracle-divergence risk
//! factors, and isolated-vs-cross-margin distinctions. This module instead
//! computes **one position's own equity/maintenance margin in isolation**,
//! using only the fields already parsed by `accounts.rs` -- good enough to
//! decide "is this specific position getting risky," not a faithful port
//! of the real multi-position engine. Do not use this for anything
//! blast-radius-sensitive beyond a single tracked position's own
//! liquidation-avoidance check.
//!
//! All math is in raw protocol units (quote lots, base lots, ticks) -- no
//! decimal/USD conversion (`Market`'s own base-lot/quote-lot-per-unit
//! conversion factors were not read in this pass, see `accounts.rs`'s
//! module doc).

use super::{PhoenixMarketState, PhoenixPositionState};

/// Mark-to-market value of one position, in raw quote lots. This is
/// exactly how the protocol itself frames P&L: `virtual_quote_lot_position`
/// is the running "quote owed/received" ledger from all fills so far, and
/// `base_lot_position * mark_price_in_quote_lots_per_base_lot` is what
/// that base-lot position is worth right now at the current mark.
pub fn unrealized_pnl_quote_lots(market: &PhoenixMarketState, position: &PhoenixPositionState) -> i64 {
    let mark_quote_lots_per_base_lot =
        (market.oracle_mark_price_ticks as i64).saturating_mul(market.tick_size as i64);
    position
        .virtual_quote_lot_position
        .saturating_add(position.base_lot_position.saturating_mul(mark_quote_lots_per_base_lot))
}

/// Funding owed (positive = owed by the trader, negative = owed to the
/// trader) since this position's last funding snapshot. `market`'s
/// `cumulative_funding_rate` is a running per-base-lot rate; the diff
/// against the position's own snapshot, scaled by position size, is the
/// funding accrued since the position was last touched.
pub fn funding_owed_since(market: &PhoenixMarketState, position: &PhoenixPositionState) -> i64 {
    let rate_diff = market
        .cumulative_funding_rate
        .saturating_sub(position.cumulative_funding_snapshot);
    rate_diff
        .saturating_mul(position.base_lot_position)
        .saturating_add(position.accumulated_funding_for_active_position)
}

/// Total account equity (collateral + unrealized P&L - funding owed) in
/// raw quote lots, for the single `position` on `market` -- see the module
/// doc for why this doesn't sum across a whole multi-position portfolio.
pub fn equity_quote_lots(
    collateral_quote_lots: i64,
    market: &PhoenixMarketState,
    position: &PhoenixPositionState,
) -> i64 {
    collateral_quote_lots
        .saturating_add(unrealized_pnl_quote_lots(market, position))
        .saturating_sub(funding_owed_since(market, position))
}

/// Simplified maintenance margin: `notional / tier0_max_leverage`, using
/// only the most permissive leverage tier (`leverage_tiers[0]`) -- not the
/// real size-scaled multi-tier formula. Returns `None` if the market isn't
/// priced yet or has no leverage configured.
pub fn maintenance_margin_quote_lots(
    market: &PhoenixMarketState,
    position: &PhoenixPositionState,
) -> Option<i64> {
    if !market.priced || market.tier0_max_leverage == 0 {
        return None;
    }
    let mark_quote_lots_per_base_lot =
        (market.oracle_mark_price_ticks as i64).saturating_mul(market.tick_size as i64);
    let notional = position.base_lot_position.saturating_abs().saturating_mul(mark_quote_lots_per_base_lot);
    Some(notional / market.tier0_max_leverage as i64)
}

/// Raw quote lots -> real USD, the same `QUOTE_LOT_DECIMALS` conversion
/// `PhoenixMarketState::mark_price_usd`/[`required_margin_usd_for_notional`]
/// already use -- exposed standalone so a caller with a raw quote-lot
/// figure that didn't come from this module (e.g. `PhoenixState::
/// collateral_quote_lots`, real currently-deposited margin) can convert it
/// to USD without duplicating the conversion factor.
pub fn quote_lots_to_usd(quote_lots: i64) -> f64 {
    quote_lots as f64 / 10f64.powi(super::QUOTE_LOT_DECIMALS)
}

/// Required PhUSD margin (real USD, not raw quote lots) for a *target*
/// notional not yet opened as a real position -- e.g. `multimodelv1`'s
/// dispersion trade needs to know how much margin a short-index leg will
/// require *before* placing the order, to decide whether to top up first.
/// Wraps [`maintenance_margin_quote_lots`] against a synthetic position of
/// the intended size (zeroed P&L/funding ledger, since it hasn't traded
/// yet) rather than reimplementing the leverage formula, then converts the
/// raw quote-lots result to USD via the same `QUOTE_LOT_DECIMALS`
/// conversion `PhoenixMarketState::mark_price_usd` already uses. `None`
/// under the same conditions `maintenance_margin_quote_lots` itself
/// returns `None` (unpriced market / no leverage configured), or if
/// `mark_price_usd()` itself is unavailable.
pub fn required_margin_usd_for_notional(market: &PhoenixMarketState, notional_usd: f64) -> Option<f64> {
    let mark_price_usd = market.mark_price_usd()?;
    if mark_price_usd <= 0.0 {
        return None;
    }
    let base_lots_per_base_unit = 10f64.powi(market.base_lot_decimals as i32);
    let num_base_lots = ((notional_usd / mark_price_usd) * base_lots_per_base_unit).round() as i64;
    let synthetic = PhoenixPositionState {
        asset_id: market.asset_id as u64,
        base_lot_position: num_base_lots,
        virtual_quote_lot_position: 0,
        cumulative_funding_snapshot: 0,
        accumulated_funding_for_active_position: 0,
    };
    let maintenance_quote_lots = maintenance_margin_quote_lots(market, &synthetic)?;
    Some(quote_lots_to_usd(maintenance_quote_lots))
}

/// `true` when equity has fallen below `threshold` times the simplified
/// maintenance margin (default caller-supplied threshold e.g. `1.5` for a
/// 50% safety buffer above the bare maintenance requirement). Conservative
/// by construction: a `None` maintenance margin (unpriced market) is
/// treated as "not at risk" rather than panicking or assuming the worst,
/// since the caller can't act on it either way until pricing arrives.
pub fn is_at_liquidation_risk(
    collateral_quote_lots: i64,
    market: &PhoenixMarketState,
    position: &PhoenixPositionState,
    threshold: f64,
) -> bool {
    let Some(maintenance) = maintenance_margin_quote_lots(market, position) else {
        return false;
    };
    if maintenance <= 0 {
        return false;
    }
    let equity = equity_quote_lots(collateral_quote_lots, market, position);
    (equity as f64) < (maintenance as f64) * threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    fn priced_market(mark_ticks: u64, tick_size: u64, max_leverage: u64) -> PhoenixMarketState {
        PhoenixMarketState {
            symbol: [0; 16],
            asset_id: 0,
            market_pk: Default::default(),
            spline_collection_pk: Default::default(),
            tick_size,
            base_lot_decimals: 0,
            tier0_max_leverage: max_leverage,
            tier0_upper_bound_size: u64::MAX,
            cumulative_funding_rate: 0,
            open_interest: 0,
            open_interest_cap: 0,
            oracle_mark_price_ticks: mark_ticks,
            priced: true,
            base_mint: None,
        }
    }

    fn flat_position(base_lots: i64) -> PhoenixPositionState {
        PhoenixPositionState {
            asset_id: 0,
            base_lot_position: base_lots,
            virtual_quote_lot_position: 0,
            cumulative_funding_snapshot: 0,
            accumulated_funding_for_active_position: 0,
        }
    }

    #[test]
    fn unrealized_pnl_is_zero_for_a_flat_position_with_no_ledger_balance() {
        let m = priced_market(100, 10, 10);
        let p = flat_position(0);
        assert_eq!(unrealized_pnl_quote_lots(&m, &p), 0);
    }

    #[test]
    fn unrealized_pnl_scales_with_position_size_and_mark_price() {
        let m = priced_market(100, 10, 10); // mark = 1000 quote-lots/base-lot
        let p = flat_position(5); // long 5 base lots, no cost basis recorded
        assert_eq!(unrealized_pnl_quote_lots(&m, &p), 5 * 1000);
    }

    #[test]
    fn funding_owed_scales_with_rate_diff_and_position_size() {
        let mut m = priced_market(100, 10, 10);
        m.cumulative_funding_rate = 50;
        let mut p = flat_position(4);
        p.cumulative_funding_snapshot = 10;
        // (50 - 10) * 4 = 160
        assert_eq!(funding_owed_since(&m, &p), 160);
    }

    #[test]
    fn liquidation_risk_flags_when_equity_falls_below_threshold_times_maintenance() {
        // Short 10 base lots, opened at the current mark (1000 quote-lots/
        // base-lot): proceeds received on entry give virtual_quote_lot_position
        // = +10_000, so a freshly-opened, unmoved position is breakeven --
        // realistic starting point, unlike an all-zero fixture.
        let m = priced_market(100, 10, 5); // mark=1000/lot, 5x max leverage
        let breakeven_short = PhoenixPositionState {
            asset_id: 0,
            base_lot_position: -10,
            virtual_quote_lot_position: 10_000,
            cumulative_funding_snapshot: 0,
            accumulated_funding_for_active_position: 0,
        };
        assert_eq!(unrealized_pnl_quote_lots(&m, &breakeven_short), 0);

        // maintenance = notional(10_000) / 5 = 2_000
        let maintenance = maintenance_margin_quote_lots(&m, &breakeven_short).unwrap();
        assert_eq!(maintenance, 2_000);

        // Comfortable collateral, breakeven position: equity == collateral,
        // well above threshold * maintenance.
        assert!(!is_at_liquidation_risk(10_000, &m, &breakeven_short, 1.5));

        // Price moves adversely against the short (mark rises to 2000/lot):
        // unrealized_pnl = 10_000 + (-10 * 2000) = -10_000. Even with the
        // same 10_000 collateral, equity (0) is now under 1.5 * maintenance.
        let m_adverse = priced_market(200, 10, 5);
        assert!(is_at_liquidation_risk(10_000, &m_adverse, &breakeven_short, 1.5));
    }

    #[test]
    fn unpriced_market_is_conservatively_not_flagged_as_at_risk() {
        let mut m = priced_market(0, 0, 0);
        m.priced = false;
        let p = flat_position(100);
        assert!(!is_at_liquidation_risk(0, &m, &p, 1.5));
    }

    // --- required_margin_usd_for_notional --------------------------------

    #[test]
    fn required_margin_usd_for_notional_matches_notional_over_leverage() {
        // mark = 100 * 10 / 1e6 = $0.001/unit, 5x max leverage.
        let m = priced_market(100, 10, 5);
        let margin = required_margin_usd_for_notional(&m, 10.0).expect("should compute a real margin");
        // notional/leverage = 10.0/5 = 2.0 -- exact at these clean numbers.
        assert!((margin - 2.0).abs() < 1e-6, "expected ~2.0, got {margin}");
    }

    #[test]
    fn required_margin_usd_for_notional_scales_linearly_with_notional() {
        let m = priced_market(100, 10, 5);
        let small = required_margin_usd_for_notional(&m, 10.0).unwrap();
        let large = required_margin_usd_for_notional(&m, 100.0).unwrap();
        assert!((large - small * 10.0).abs() < 1e-3, "expected {large} ~= {} * 10", small);
    }

    #[test]
    fn required_margin_usd_for_notional_none_when_unpriced() {
        let mut m = priced_market(100, 10, 5);
        m.oracle_mark_price_ticks = 0;
        assert!(required_margin_usd_for_notional(&m, 10.0).is_none());
    }

    #[test]
    fn required_margin_usd_for_notional_none_when_no_leverage_configured() {
        let m = priced_market(100, 10, 0);
        assert!(required_margin_usd_for_notional(&m, 10.0).is_none());
    }
}
