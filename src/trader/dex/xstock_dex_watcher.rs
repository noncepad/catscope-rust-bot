//! Watches the single most liquid Orca Whirlpool pool per xStock ticker,
//! directly (same `depth: 0` direct-subscription pattern as
//! `kamino_xstocks_watcher.rs`, not the account graph). Purpose: a live
//! activity signal, not a trading strategy -- real DEX trading against
//! these tokens is far more frequent than Kamino-specific events (reserve
//! refreshes, obligation touches), and it's a genuine leading indicator:
//! Scope's own oracle sources are informed by real trading activity
//! across venues, so a real price move here can show up before Scope's
//! own ~30-40s refresh catches up. Complements
//! `KaminoXstocksWatcher::live_price_usd`, doesn't replace it -- this
//! bot's liquidation eligibility still runs off Scope, not this.
//!
//! # Pool selection
//!
//! One pool per ticker, picked from a real `getProgramAccounts`-derived
//! Orca Whirlpool snapshot: prefer the USDC-quoted pool, highest vault
//! balance, over a SOL-quoted one. All 10 real xStocks turned out to have
//! a USDC pool. Live-verified end to end against one of them (not
//! assumed): SPYx/USDC's real `sqrt_price_x64` decoded to $773.56,
//! matching Kamino's own live Scope price ($771.77) within ~0.2%, exactly
//! what two independent price sources for the same real asset should
//! look like.
//!
//! # Orientation and decimals
//!
//! Every one of the 10 chosen pools has the xStock as `mint_a` and USDC
//! as `mint_b` (verified against the same snapshot, not assumed to hold
//! generally). xStock mints are 8 decimals, USDC is 6 (`mint_info.json`
//! this session) -- so `OrcaWhirlpool::spot_price()` (raw token_b per
//! raw token_a) needs `* 10^(8-6)` to become a real USD price. See
//! [`spot_price_usd`].

use std::collections::{HashMap, VecDeque};

use crate::{
    catscope::witbot::shooter::Header,
    graph::{AccountId, Graph, Subscription, SubscriptionQueue, SubscriptionRequest},
    util::account_id_from_pubkey,
};
use solana_sdk::{clock::Slot, pubkey::Pubkey};

use super::orca::{parse as parse_whirlpool, ORCA_WHIRLPOOL_PROGRAM_ID};

/// (ticker, pool pubkey) -- see this module's own doc comment for how
/// these were picked and verified. xStock is always `mint_a`, USDC always
/// `mint_b` in every one of these.
const XSTOCK_DEX_POOLS: &[(&str, &str)] = &[
    ("HOODx", "9rC9wbXD16odLdNzhgk1nacZwXJPRn74auDNmFhSJLZ7"),
    ("METAx", "59qH56HhMXrXiCMWH46NZ5M3SWhyzVzZnvMP2R27aR7Z"),
    ("AAPLx", "5S3NUMbm8aX6Jvj7TymRHwbP54ae7TX13Q7iFx5p1dK"),
    ("NVDAx", "6R4r93V5fcMzc13CL2enEepDSYcr4Qx3ptZBDwudTXCo"),
    ("TSLAx", "9p7abUFv31ycgu9kckvnoqMMvBy67dqTDM2m6HP9xokN"),
    ("GOOGLx", "FaGxc8NXSXBrT6idTxjw3o8et4MmFjxRMQvVF7ChsgZV"),
    ("QQQx", "3GVB4bXtcrP3MM376mrcJDwfTNThvyorLmVgSTf6kxFt"),
    ("SPYx", "Fae5dWVntUt6zbWu2voXxioDpMii7SqQwtsxBmoVCsHR"),
    ("CRCLx", "9fhWexdQMvuH8dkAvCCyaBWTX3AoKYYQuBXWE5Xeekjb"),
    ("MSTRx", "CHSijZ92W1A5z93WzGjvBygtxN2LWgAt58uejMDmFVpY"),
];

/// xStock mints are 8 decimals (`mint_info.json`, this session).
const XSTOCK_MINT_DECIMALS: i32 = 8;
/// USDC is 6 decimals (canonical, cross-checked the same way).
const USDC_MINT_DECIMALS: i32 = 6;

/// Real USD price from a Whirlpool's raw spot price -- see this module's
/// own doc comment for the decimal math and the live cross-check against
/// Kamino's Scope price that confirmed it.
fn spot_price_usd(raw_spot: f64) -> f64 {
    raw_spot * 10f64.powi(XSTOCK_MINT_DECIMALS - USDC_MINT_DECIMALS)
}

/// One live price observation -- pushed to [`XstockDexWatcher::recent`]
/// whenever a tracked pool's price actually changes (not on every push;
/// a Whirlpool account can be touched by things that don't move price,
/// e.g. a fee-collection instruction).
#[derive(Debug, Clone, Copy)]
pub struct DexTick {
    pub ticker: &'static str,
    pub price_usd: f64,
    /// `true` if this price is higher than the previous observation for
    /// this pool, `false` if lower. Meaningless (arbitrarily `true`) on
    /// the very first observation for a pool, since there's no previous
    /// price to compare against.
    pub up: bool,
    pub slot: Slot,
}

/// Hard cap on how many recent ticks this watcher keeps in memory / sends
/// on the wire -- same reasoning as `KaminoXstocksWatcher::
/// HEALTH_BOARD_MAX_ENTRIES`, a small, recent activity feed is the point,
/// not an unbounded log.
const RECENT_CAP: usize = 20;

/// Live-tracked state for [`XSTOCK_DEX_POOLS`]. See this module's own doc
/// comment.
#[derive(Debug, Default)]
pub struct XstockDexWatcher {
    /// Pool AccountId -> ticker, so `on_account` knows both "is this one
    /// of mine" and which ticker it is in one lookup.
    s_pool_id: HashMap<AccountId, &'static str>,
    m_last_price_usd: HashMap<AccountId, f64>,
    /// Most recent ticks, newest last -- see [`RECENT_CAP`].
    recent: VecDeque<DexTick>,
    sub_queue: SubscriptionQueue,
    _subscriptions: Vec<Subscription>,
}

impl XstockDexWatcher {
    pub fn new() -> Self {
        let s_pool_id: HashMap<AccountId, &'static str> = XSTOCK_DEX_POOLS
            .iter()
            .map(|(ticker, pk)| (account_id_from_pubkey(&Pubkey::from_str_const(pk)), *ticker))
            .collect();

        let mut sub_queue = SubscriptionQueue::default();
        sub_queue.extend(s_pool_id.keys().map(|&id| SubscriptionRequest {
            root: id,
            filter_weight: 0,
            depth: 0,
        }));

        Self {
            s_pool_id,
            m_last_price_usd: HashMap::new(),
            recent: VecDeque::with_capacity(RECENT_CAP),
            sub_queue,
            _subscriptions: Vec::new(),
        }
    }

    pub fn pending_subscriptions(&self) -> usize {
        self.sub_queue.pending_count()
    }

    pub fn flush_subscriptions(
        &mut self,
        g: &Graph,
        max_per_flush: usize,
    ) -> Result<usize, crate::err::CatscopeGuestError> {
        self.sub_queue.flush(g, max_per_flush)
    }

    pub fn on_account(&mut self, header: &Header, body: &[u8]) {
        let Some(&ticker) = self.s_pool_id.get(&header.accountid) else {
            return;
        };
        // Same real-data guard as KaminoXstocksWatcher::on_account -- a
        // subscription to a not-yet-existing or since-closed account
        // still produces exactly one push (owner = System Program,
        // lamports == 0); don't parse that as a real price.
        let is_real = header.owner == account_id_from_pubkey(&ORCA_WHIRLPOOL_PROGRAM_ID)
            && header.lamports > 0;
        if !is_real {
            return;
        }
        let Some(pool) = parse_whirlpool(body) else {
            return;
        };
        let price_usd = spot_price_usd(pool.spot_price());
        if !price_usd.is_finite() || price_usd <= 0.0 {
            return;
        }
        let up = self
            .m_last_price_usd
            .get(&header.accountid)
            .is_none_or(|&prev| price_usd >= prev);
        // Only record a tick when the price actually moved -- a Whirlpool
        // account can be touched (fee collection, position open/close)
        // without its price changing at all, and those aren't the "real
        // trade happened" signal this feed is for.
        let moved = self
            .m_last_price_usd
            .get(&header.accountid)
            .is_none_or(|&prev| prev != price_usd);
        self.m_last_price_usd.insert(header.accountid, price_usd);
        if !moved {
            return;
        }
        if self.recent.len() >= RECENT_CAP {
            self.recent.pop_front();
        }
        self.recent.push_back(DexTick { ticker, price_usd, up, slot: header.slot });
    }

    /// Most recent ticks, newest last -- see [`RECENT_CAP`].
    pub fn recent(&self) -> impl Iterator<Item = &DexTick> {
        self.recent.iter()
    }

    /// This pool's most recently observed real price for `ticker`, if
    /// any -- for the arb-detection tracker in `state.rs`, which needs
    /// to compare this against `KaminoXstocksWatcher::
    /// live_price_usd_by_ticker`'s own number for the same ticker. Small
    /// linear scan over `s_pool_id` (10 entries) rather than a second
    /// ticker-keyed map -- not worth the extra bookkeeping at this size.
    pub fn price_usd_by_ticker(&self, ticker: &str) -> Option<f64> {
        let (&pool_id, _) = self.s_pool_id.iter().find(|(_, &t)| t == ticker)?;
        self.m_last_price_usd.get(&pool_id).copied()
    }

    /// Total number of pools this watcher subscribes to (a fixed
    /// constant, [`XSTOCK_DEX_POOLS`].len() = 10) -- for the dashboard's
    /// "N pools monitored" stat.
    pub fn pool_count() -> usize {
        XSTOCK_DEX_POOLS.len()
    }
}
