//! Lending-market borrow/collateral capacity, modeled separately from
//! [`crate::trader::pricegraph::TradeRouter`]'s swap-price edges.
//!
//! A lending reserve/bank isn't a priced A↔B conversion the way an AMM pool
//! is: depositing collateral and borrowing against it doesn't settle at an
//! exchange rate, it's bounded by the collateral asset's loan-to-value
//! ratio, costs an ongoing borrow rate (not a one-time swap fee), and is
//! capped by how much liquidity actually sits in the borrow-side reserve.
//! Each reserve plays two independent roles -- as *collateral* (its own
//! LTV) and as a *borrow source* (its own rate + available liquidity) --
//! so a "credit edge" from one asset to another is the combination of one
//! reserve's collateral side with a different reserve's borrow side within
//! the same lending market.
//!
//! This module deliberately does not plug into `TradeRouter`'s
//! Bellman-Ford graph; composing the two (e.g. "deposit A, borrow B, then
//! swap B→C via `TradeRouter::route`") is left to a future planner once
//! more than one protocol has real numeric data (see the credit-graph plan
//! for phasing).

use crate::graph::AccountId;

/// One reserve's lending-relevant numbers, usable either as a collateral
/// source or a borrow source (most reserves can be both).
#[derive(Debug, Clone, Copy)]
pub struct CreditReserve {
    pub reserve_id: AccountId,
    pub mint: AccountId,
    /// Fraction (0.0-1.0) of this asset's USD value that can be borrowed
    /// against when deposited as collateral. `0.0` means this reserve
    /// cannot be used as collateral at all (e.g. isolated/borrow-only).
    pub max_ltv_pct: f64,
    /// Ongoing cost (fraction per year) to borrow this asset. `0.0` if
    /// unknown -- callers should treat that as "unpriced", not "free".
    pub borrow_apy: f64,
    /// USD value of liquidity actually sitting in this reserve, i.e. the
    /// hard cap on how much of it can be borrowed right now.
    pub available_liquidity_usd: f64,
}

impl CreditReserve {
    /// Build a `CreditReserve` from a parsed Kamino reserve. `reserve_id`
    /// is the reserve account's own pubkey (not stored on `KaminoReserve`
    /// itself).
    pub fn from_kamino(reserve_id: AccountId, r: &super::dex::kamino::KaminoReserve) -> Self {
        let raw_to_usd = r.price_usd / 10f64.powi(r.mint_decimals as i32);
        Self {
            reserve_id,
            mint: r.token_mint,
            max_ltv_pct: r.loan_to_value_pct,
            borrow_apy: r.current_borrow_apy(),
            available_liquidity_usd: r.available_amount as f64 * raw_to_usd,
        }
    }

    /// Build a `CreditReserve` from a parsed marginfi-v2 Bank. `reserve_id`
    /// is the Bank account's own pubkey (not stored on `MarginfiBank`
    /// itself).
    ///
    /// `price_usd` comes from `MarginfiState::credit_reserve`, which only
    /// has one to give when the bank's `oracle_setup` is Pyth legacy *and*
    /// an update has actually arrived for its oracle account -- pass
    /// `None` (leaving `available_liquidity_usd` at `0.0`, unpriced) for
    /// every other case (Fixed, Switchboard, other-protocol oracles, or
    /// simply no update seen yet).
    pub fn from_marginfi(
        reserve_id: AccountId,
        b: &super::dex::marginfi::MarginfiBank,
        price_usd: Option<f64>,
    ) -> Self {
        let available_liquidity_usd = match price_usd {
            Some(p) => b.available_liquidity_tokens() * p,
            None => 0.0,
        };
        Self {
            reserve_id,
            mint: b.mint,
            max_ltv_pct: b.asset_weight_init,
            borrow_apy: b.current_borrow_apy(),
            available_liquidity_usd,
        }
    }

    /// Build a `CreditReserve` from a parsed Solend Reserve. `reserve_id`
    /// is the reserve account's own pubkey (not stored on `SolendReserve`
    /// itself). Unlike marginfi, Solend stores its own oracle price and a
    /// full borrow-rate curve directly in the account, so both
    /// `available_liquidity_usd` and `borrow_apy` are fully priced here.
    pub fn from_solend(reserve_id: AccountId, r: &super::dex::solend::SolendReserve) -> Self {
        let raw_to_usd = r.price_usd / 10f64.powi(r.mint_decimals as i32);
        Self {
            reserve_id,
            mint: r.mint,
            max_ltv_pct: r.loan_to_value_pct,
            borrow_apy: r.current_borrow_apy(),
            available_liquidity_usd: r.available_amount as f64 * raw_to_usd,
        }
    }

    /// Build a `CreditReserve` from a parsed Drift SpotMarket. `reserve_id`
    /// is the market account's own pubkey (not stored on `DriftSpotMarket`
    /// itself). `price_usd` here is an oracle snapshot, not a live read --
    /// see the module doc comment on `dex::drift` for the caveat.
    pub fn from_drift(reserve_id: AccountId, m: &super::dex::drift::DriftSpotMarket) -> Self {
        let raw_to_usd = m.price_usd / 10f64.powi(m.mint_decimals as i32);
        Self {
            reserve_id,
            mint: m.mint,
            max_ltv_pct: m.initial_asset_weight,
            borrow_apy: m.current_borrow_apy(),
            available_liquidity_usd: m.available_amount() * raw_to_usd,
        }
    }

    /// Build a `CreditReserve` from a Jet Protocol V1 reserve. Unlike the
    /// other `from_*` constructors, this needs `mint` passed in separately
    /// (from the reserve's own address-book entry) since Jet's cached
    /// per-reserve info -- the only part of this protocol reliably parsed
    /// so far, see the module doc comment on `dex::jet` -- doesn't include
    /// it. `available_liquidity_usd` and `borrow_apy` are left at `0.0`
    /// (unpriced) for the same reason.
    pub fn from_jet(reserve_id: AccountId, mint: AccountId, info: &super::dex::jet::JetReserveInfo) -> Self {
        Self {
            reserve_id,
            mint,
            max_ltv_pct: info.max_ltv_pct(),
            borrow_apy: 0.0,
            available_liquidity_usd: 0.0,
        }
    }
}

/// All reserves within one lending market (Kamino `lending_market`,
/// MarginFi `group`, Solend `lending_market`, Jet `market` -- Drift has no
/// per-market grouping since it's a single global program).
#[derive(Debug, Clone, Default)]
pub struct CreditMarket {
    pub market_id: AccountId,
    pub reserves: Vec<CreditReserve>,
}

impl CreditMarket {
    pub fn new(market_id: AccountId) -> Self {
        Self {
            market_id,
            reserves: Vec::new(),
        }
    }

    /// USD borrowable of `borrow`'s asset against `collateral_usd` worth of
    /// `collateral`'s asset, capped by however much liquidity `borrow`
    /// actually has available. Returns `0.0` if `collateral` can't be used
    /// as collateral at all (`max_ltv_pct == 0.0`).
    pub fn max_borrow_usd(
        &self,
        collateral: &CreditReserve,
        borrow: &CreditReserve,
        collateral_usd: f64,
    ) -> f64 {
        (collateral_usd * collateral.max_ltv_pct).min(borrow.available_liquidity_usd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixture values taken from real mainnet Kamino Reserve accounts
    /// (fetched and offset-verified during this session): a SOL reserve at
    /// 70% LTV with ~$50k available liquidity, and a USDC reserve usable
    /// only as a borrow source (0% LTV -- an isolated/borrow-only market)
    /// with ~$1M available liquidity.
    fn sol_collateral() -> CreditReserve {
        CreditReserve {
            reserve_id: 1,
            mint: 100,
            max_ltv_pct: 0.70,
            borrow_apy: 0.0,
            available_liquidity_usd: 50_000.0,
        }
    }

    fn usdc_borrow() -> CreditReserve {
        CreditReserve {
            reserve_id: 2,
            mint: 200,
            max_ltv_pct: 0.0,
            borrow_apy: 0.0,
            available_liquidity_usd: 1_000_000.0,
        }
    }

    #[test]
    fn max_borrow_respects_ltv() {
        let market = CreditMarket::new(1);
        let sol = sol_collateral();
        let usdc = usdc_borrow();
        // $1000 of SOL collateral at 70% LTV unlocks $700 of USDC borrowing
        // power, and the USDC reserve has plenty of liquidity to cover it.
        assert_eq!(market.max_borrow_usd(&sol, &usdc, 1_000.0), 700.0);
    }

    #[test]
    fn max_borrow_capped_by_available_liquidity() {
        let market = CreditMarket::new(1);
        let sol = sol_collateral();
        let mut usdc = usdc_borrow();
        // Only $500 of USDC actually sits in the reserve -- can't borrow
        // more than that even though LTV would otherwise allow $700.
        usdc.available_liquidity_usd = 500.0;
        assert_eq!(market.max_borrow_usd(&sol, &usdc, 1_000.0), 500.0);
    }

    #[test]
    fn zero_ltv_reserve_cannot_be_used_as_collateral() {
        let market = CreditMarket::new(1);
        let usdc = usdc_borrow(); // max_ltv_pct == 0.0
        let sol = sol_collateral();
        assert_eq!(market.max_borrow_usd(&usdc, &sol, 1_000.0), 0.0);
    }

    fn marginfi_bank_fixture() -> crate::trader::dex::marginfi::MarginfiBank {
        crate::trader::dex::marginfi::MarginfiBank {
            mint: 300,
            mint_decimals: 6,
            group: 1,
            asset_share_value: 1.0,
            liability_share_value: 1.0,
            total_asset_shares: 1_000.0,
            total_liability_shares: 400.0,
            asset_weight_init: 0.8,
            oracle_setup: 1,
            oracle_key: 400,
            ..Default::default()
        }
    }

    #[test]
    fn from_marginfi_prices_liquidity_when_price_known() {
        let bank = marginfi_bank_fixture();
        let cr = CreditReserve::from_marginfi(5, &bank, Some(2.0));
        // (1000 - 400) raw units / 10^6 decimals = 0.0006 tokens, at $2 each.
        assert!((cr.available_liquidity_usd - 0.0012).abs() < 1e-12);
        assert_eq!(cr.max_ltv_pct, 0.8);
    }

    #[test]
    fn from_marginfi_stays_unpriced_without_a_price() {
        let bank = marginfi_bank_fixture();
        let cr = CreditReserve::from_marginfi(5, &bank, None);
        assert_eq!(cr.available_liquidity_usd, 0.0);
    }
}
