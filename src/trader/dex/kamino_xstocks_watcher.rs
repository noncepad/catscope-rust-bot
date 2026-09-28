//! Watches a set of known Kamino Obligation accounts, plus the xStocks
//! market's own reserves, directly -- one `depth: 0` subscription per
//! account -- rather than through edge-generator's Catscope account
//! graph. See `kamino.rs`'s `refresh_obligation` doc comment for the full
//! reasoning: this bot has never watched other users' positions before
//! (everything else in this module tracks only the bot's own), so whether
//! to do that at all is a scope decision, not a technical one, and is
//! being kept separate from this file. This file is the technical piece:
//! given a list of obligation and reserve addresses, track their live
//! contents and compute a health factor for each obligation.
//!
//! # Why direct subscription, not the graph
//!
//! A `depth: 0` subscription needs no edge to exist for the target
//! account -- only that it's already inside the tracked universe, which
//! both account types are (owned by Kamino's lending program, already in
//! `kamino.rs`'s own `program_id_list()`-equivalent tracking). This
//! trades away *live* discovery of new obligations (the list this file
//! subscribes to is a build-time snapshot, refreshed by re-running
//! `download-arb` -- see `crate::kamino_xstocks_obligation_config`'s doc
//! comment -- not something that grows as new obligations open on-chain
//! after this binary was built) for not touching edge-generator at all.
//!
//! # Why paced, not one `bulk_subscribe` call
//!
//! [`SubscriptionQueue`] is used here, not a single `bulk_subscribe` --
//! `subscribe`/`bulk_subscribe` are blocking calls on the validator side,
//! and this codebase has already seen a real ~32,000-account startup
//! burst (Raydium/Orca/lending-reserve pools) block on exactly that. This
//! file's list is far smaller, but the pattern is "always page it,"
//! not "page it once it's big enough to matter."

use std::collections::{HashMap, HashSet};

use crate::{
    catscope::witbot::shooter::Header,
    graph::{AccountId, Graph, Subscription, SubscriptionQueue, SubscriptionRequest},
    util::account_id_from_pubkey,
};
use solana_sdk::pubkey::Pubkey;

use super::kamino::{
    parse as parse_kamino_reserve, parse_kamino_obligation, KaminoObligation, KaminoReserve,
    KAMINO_LENDING_PROGRAM_ID, SCOPE_PROGRAM_ID,
};

/// Fallback sample of real obligation addresses in Kamino's xStocks
/// lending market (`5wJeMrUYECGq41fxRESKALVcHnNX26TAWy4W98yULsua`),
/// originally fetched directly via `getProgramAccounts` (memcmp on
/// Obligation's real discriminator -- `[168, 206, 141, 106, 88, 76, 172,
/// 167]`, from klend-sdk's own IDL-generated TS, not computed by hand --
/// plus `lending_market` at offset 32), 2026-09-17. Real and
/// independently confirmed: each of these decoded with a
/// `deposits[0].deposit_reserve` matching a real xStocks-market reserve
/// address for the ticker named in its comment.
///
/// No longer the primary source -- `KaminoXstocksWatcher::new` now prefers
/// `crate::kamino_xstocks_obligation_config::XSTOCKS_OBLIGATIONS_GENERATED`,
/// the real, `optimizer`-scanned full list (thousands of addresses, not
/// 8; see that module's doc comment). This hand-picked 8-of-~6,968 sample
/// stays as the fallback for a dev build whose prefetch.db predates that
/// scan (or hasn't re-run `download-arb` since it was added), so `cargo
/// build` never silently produces a bot with zero obligations to watch.
const XSTOCKS_OBLIGATIONS: &[&str] = &[
    "12CabEqeJMYDC51tc4henPtUB3et3nDKk2AaF9EPg3KJ", // NVDAx collateral
    "12KUWouLkhFDnAXxx8wqdH7UtvqYoh7ZsG3LRV2xyUPg", // TSLAx collateral
    "12Q241ZGxVBEWaaH2qpw7EZqn3G5ZPbDF9wDXGHNAoPd", // SPYx collateral
    "12X3daXhES7RwDh64aojQNT42c8w5kJPboesmmVi1nT5", // NVDAx collateral
    "12dB2KbtSEg7r5cDdi3MFqpFomhMC6mPYuDiZD6yjjAZ", // TSLAx collateral
    "12rtz1KbjWUnr88b2D83zXfLV29eyd2C2XWo8eVZDmLA", // SPYx collateral
    "1345WByFGa9JzSjGwJaFxGwyFqfVe392caQxEhjfMStc", // SPYx collateral
    "13WHwLXaHzCL4R6w7k3TyoZWnQVe18ypZ8kjaDx7jGD1", // SPYx collateral
];

/// The 10 real xStock reserves in the same market, from the hackathon
/// plan's own research pass (`HACKATHON_PLAN.md` §3a) -- independently
/// re-confirmed against every obligation in [`XSTOCKS_OBLIGATIONS`] above
/// (each one's `deposit_reserve` matches one of these). Needed alongside
/// the obligations themselves: an obligation's raw bytes alone don't
/// carry price or liquidation-threshold data, only the reserves it
/// references do.
const XSTOCKS_RESERVES: &[(&str, &str)] = &[
    ("UvXjBuC7YZYaGB9Rn1PpBD1GySmjzunXgE8Zev9ua8d", "SPYx"),
    ("2jerdAXR8r2B6z3P7P6VgSiePQX7wqcpbEqdDbm8mgeB", "QQQx"),
    ("4UBJu5Xp1aziV9frBQBhc1RnKrgXHAWHYejQytkYr8gq", "HOODx"),
    ("4wg6rEkGgHaEuxMduP46C1xFZ24Lnp5YgdNkZAHxFzsN", "GOOGLx"),
    ("57qagnQFuWw1seEqi6Z5JBvkm5xH5svdmq9dtqxG1rYy", "CRCLx"),
    ("5iTiczqgUegqA3PpoNpotizMbY9n1sRWr3oL6igKvWuf", "TSLAx"),
    ("7B66Az3tJhAo4bLkX8PzTixQ9ZGyHkkjxfVLhF26sP5q", "NVDAx"),
    ("CKJbqakbPGyhziowm19LPYz636UszuezfkitmpRtcLSH", "AAPLx"),
    ("AJPrye7NZGex2rUZhRwiAPJYxai1Ptb7DNWR3yYjtk3G", "METAx"),
    ("Cwy2WJoswCMyfPtWTrmiaDLXC3phz3qwr1TaT4kaSAyD", "MSTRx"),
];

/// The market's 3 real borrow-side (debt) reserves -- USDC, cbBTC, USDG,
/// confirmed on-chain this session (`getProgramAccounts` on the market,
/// mint field decoded for each of the 3 non-xStock reserves found: USDC's
/// mint is the well-known canonical one, cbBTC's mint literally starts
/// with "cbbtc", USDG confirmed via web search as "Global Dollar").
///
/// Real bug this fixes: [`XSTOCKS_RESERVES`] above only lists the 10
/// *collateral*-side reserves, so before this const existed, `health()`
/// could never price any obligation's debt at all -- every real
/// debt-bearing obligation was silently excluded from
/// [`KaminoXstocksWatcher::health_board`] as `complete: false` (missing
/// the borrow-side reserve's live data), regardless of how underwater it
/// actually was. Live-tested runs this session only ever showed
/// zero-debt obligations as "complete" as a direct result -- a liquidator
/// that can never see real debt can never find anything real to
/// liquidate.
const XSTOCKS_DEBT_RESERVES: &[(&str, &str)] = &[
    ("97zoywd8mPZsGTg8q1wdD2Wgkdrs2tqusp1Qqcxbyj7E", "USDC"),
    ("5AWpVYJvNASoUM8toSDQRcVyA9dvoUuFj5qNx9v32bjj", "cbBTC"),
    ("F1xMZ8em6SrQkCnKQR1pzcxQieSUth35sYDQ2kK6o8tX", "USDG"),
];

/// The single Scope `OraclePrices` account every one of the 13 xStocks-
/// market reserves' `scope_prices` field points at -- live-verified via a
/// real `getMultipleAccounts` call against all 13 reserves this session,
/// not assumed from one. Watching this one account gives a live price
/// for every reserve here, each identified by its own
/// `KaminoReserve::scope_price_chain_index` -- see `kamino::
/// parse_scope_price`'s own doc comment for the real on-chain layout and
/// why this matters (this account refreshes on Scope's own ~30-40s
/// cadence, 24/7, independent of whether anyone's sent a Kamino
/// instruction against any specific reserve recently).
pub const XSTOCKS_SCOPE_PRICES_ACCOUNT: &str = "3t4JZcueEzTbVP6kLxXrL3VpWx45jDer4eqysweBchNH";

/// Reverse lookup: real ticker string for a known reserve `AccountId`, or
/// `"?"` if it isn't one of the 13 tracked reserves. Used for the
/// liquidation-event outbound message (`message.rs`'s
/// `CustomMessageOutbound::LiquidationEvent`) so the dashboard can show
/// "TSLAx"/"USDC" instead of a raw internal account id.
/// The 10 xStock collateral tickers (not the 3 debt-side ones) -- for
/// callers (the arb-detection tracker in `state.rs`) that need to walk
/// every ticker without reaching into [`XSTOCKS_RESERVES`] directly.
pub fn xstock_tickers() -> impl Iterator<Item = &'static str> {
    XSTOCKS_RESERVES.iter().map(|(_, ticker)| *ticker)
}

pub fn reserve_ticker(reserve_id: AccountId) -> &'static str {
    XSTOCKS_RESERVES
        .iter()
        .chain(XSTOCKS_DEBT_RESERVES.iter())
        .find(|(s, _)| account_id_from_pubkey(&Pubkey::from_str_const(s)) == reserve_id)
        .map(|(_, ticker)| *ticker)
        .unwrap_or("?")
}

/// Same lookup as [`reserve_ticker`], but returns this reserve's position
/// in the combined `XSTOCKS_RESERVES` + `XSTOCKS_DEBT_RESERVES` list
/// (0..=9 = SPYx,QQQx,HOODx,GOOGLx,CRCLx,TSLAx,NVDAx,AAPLx,METAx,MSTRx;
/// 10..=12 = USDC,cbBTC,USDG) instead of the ticker string itself. Used
/// for `ObligationHealth::collateral_ticker_id`/`debt_ticker_id` -- a
/// 1-byte index is cheap enough to put on the `HealthBoard` wire message
/// per-entry (100 entries) without threatening the 4096-byte ceiling
/// `HEALTH_BOARD_MAX_ENTRIES`'s doc comment describes; an 8-byte ticker
/// string per field, at that entry count, already broke that ceiling once
/// (see `LiquidationEvent`'s own ticker fields for why *that* message
/// could afford full strings -- it only ever carries one entry, not 100).
/// The Go side must decode with the exact same index order -- see
/// `optimizer/brain/xstockshealthv1/message.go`'s `tickerTable`. 255 is
/// the "no position found" sentinel (should only happen for an obligation
/// `health()` already marks incomplete).
fn reserve_ticker_id(reserve_id: AccountId) -> u8 {
    XSTOCKS_RESERVES
        .iter()
        .chain(XSTOCKS_DEBT_RESERVES.iter())
        .position(|(s, _)| account_id_from_pubkey(&Pubkey::from_str_const(s)) == reserve_id)
        .map(|i| i as u8)
        .unwrap_or(255)
}

/// Same ticker-table index as [`reserve_ticker_id`], but keyed by the
/// ticker string itself rather than a reserve `AccountId` -- for callers
/// (`xstock_dex_watcher`) that only have the ticker, not a Kamino
/// reserve account. Same table, same index order, so the Go side's
/// existing `tickerTable`/`tickerName` (built for `HealthBoard`) decodes
/// this without needing a second table.
pub fn ticker_to_id(ticker: &str) -> u8 {
    XSTOCKS_RESERVES
        .iter()
        .chain(XSTOCKS_DEBT_RESERVES.iter())
        .position(|(_, t)| *t == ticker)
        .map(|i| i as u8)
        .unwrap_or(255)
}

/// The real Kamino xStocks lending market -- see this module's own doc
/// comment. Exposed as a reusable `Pubkey` constant (not just a string in
/// [`XSTOCKS_RESERVES`]) for other code that needs to reference this
/// specific market directly, e.g. `testperpv1`'s real TSLAx-into-Kamino
/// deposit/withdraw test (see `kamino::deposit_with_token_program`'s doc
/// comment for why that test needed a Token-2022-aware deposit/withdraw
/// in the first place -- this reserve's own liquidity mint, TSLAx, is
/// real Token-2022).
pub const XSTOCKS_LENDING_MARKET: Pubkey =
    Pubkey::from_str_const("5wJeMrUYECGq41fxRESKALVcHnNX26TAWy4W98yULsua");
/// TSLAx's real reserve in [`XSTOCKS_LENDING_MARKET`] -- same address as
/// the `"TSLAx"` entry in [`XSTOCKS_RESERVES`], exposed as a `Pubkey`
/// constant for the same reason as that constant.
pub const TSLAX_RESERVE: Pubkey = Pubkey::from_str_const("5iTiczqgUegqA3PpoNpotizMbY9n1sRWr3oL6igKvWuf");

/// A computed health snapshot for one obligation. `collateral_usd` is
/// already weighted by each collateral reserve's `liquidation_threshold_pct`
/// (i.e. it's the *liquidation* value, not the raw deposit value) --
/// `health_factor <= 1.0` means eligible for liquidation right now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ObligationHealth {
    pub collateral_usd: f64,
    pub debt_usd: f64,
    /// `f64::INFINITY` when `debt_usd == 0.0` (fully repaid / collateral-only).
    pub health_factor: f64,
    /// `false` if any reserve this obligation references hasn't delivered
    /// a live update yet -- `collateral_usd`/`debt_usd`/`health_factor`
    /// only reflect the reserves that *have*, so a `false` result should
    /// be treated as "not yet known," not "healthy."
    pub complete: bool,
    /// [`reserve_ticker_id`] of the single largest collateral deposit by
    /// USD value (same "largest position" pick `state.rs`'s
    /// `attempt_liquidate` uses) -- an obligation can hold up to 8
    /// deposits at once, this is just the dominant one, good enough for a
    /// one-ticker-per-row dashboard column. 255 if none found.
    pub collateral_ticker_id: u8,
    /// Same as `collateral_ticker_id` but for the largest borrow by USD
    /// value.
    pub debt_ticker_id: u8,
}

/// Live-tracked contents of a set of known Kamino obligations plus the
/// reserves needed to price them.
#[derive(Debug, Default)]
pub struct KaminoXstocksWatcher {
    m_obligation: HashMap<AccountId, KaminoObligation>,
    m_reserve: HashMap<AccountId, KaminoReserve>,
    /// Which known accounts are reserves, not obligations -- `on_account`
    /// needs this to know which parser to try; a reserve and an
    /// obligation are different fixed sizes, but checking membership here
    /// is cheaper and doesn't depend on that staying true.
    s_reserve_id: HashSet<AccountId>,
    /// The subset of `s_reserve_id` that are xStock (collateral-side, Token-2022
    /// liquidity mint) reserves rather than debt-side (classic SPL) ones --
    /// see [`Self::liquidity_token_program`]'s doc comment for what this is
    /// used for and how confident that Token-2022-for-every-xStock
    /// assumption actually is.
    s_xstock_reserve_id: HashSet<AccountId>,
    /// [`XSTOCKS_SCOPE_PRICES_ACCOUNT`], precomputed once -- `on_account`
    /// needs to recognize it every call.
    scope_prices_id: AccountId,
    /// Raw bytes of [`XSTOCKS_SCOPE_PRICES_ACCOUNT`]'s most recent real
    /// update -- `None` until the first push arrives (or if it's ever
    /// closed/reinitialized to non-Kamino-Lending-owned data, same
    /// `is_real` convention `on_account` already uses for reserves/
    /// obligations, though in practice a Scope account this central
    /// should never actually go away). Read by [`Self::health`] via
    /// `KaminoReserve::live_price_usd`; kept as raw bytes rather than
    /// pre-decoding into 13 separate prices since [`kamino::
    /// parse_scope_price`] is cheap and each reserve needs a different
    /// index anyway.
    o_scope_prices_body: Option<Vec<u8>>,
    sub_queue: SubscriptionQueue,
    /// Keeps every sent subscription alive -- same "just don't let it
    /// drop" role every other `_subscriptions: Vec<Subscription>` field
    /// in this codebase plays (see `SubscriptionQueue`'s own doc comment).
    _subscriptions: Vec<Subscription>,
}

impl KaminoXstocksWatcher {
    pub fn new() -> Self {
        let s_reserve_id: HashSet<AccountId> = XSTOCKS_RESERVES
            .iter()
            .chain(XSTOCKS_DEBT_RESERVES.iter())
            .map(|(s, _ticker)| account_id_from_pubkey(&Pubkey::from_str_const(s)))
            .collect();

        // Prefer the real, optimizer-scanned full list over the small
        // hand-picked XSTOCKS_OBLIGATIONS sample -- see both consts' own
        // doc comments. Falls back to the sample only when the generated
        // list is empty.
        let generated = crate::kamino_xstocks_obligation_config::XSTOCKS_OBLIGATIONS_GENERATED;
        let obligation_ids: Vec<AccountId> = if !generated.is_empty() {
            generated
                .iter()
                .map(|raw| account_id_from_pubkey(&Pubkey::new_from_array(*raw)))
                .collect()
        } else {
            XSTOCKS_OBLIGATIONS
                .iter()
                .map(|s| account_id_from_pubkey(&Pubkey::from_str_const(s)))
                .collect()
        };

        let mut sub_queue = SubscriptionQueue::default();
        // Reserves queued FIRST, obligations second -- SubscriptionQueue is
        // a plain FIFO (VecDeque), and every single health() computation
        // needs its referenced reserves' live data regardless of which
        // obligation it's for. With only ~10 reserves against thousands of
        // obligations, queuing them last (as an earlier version of this
        // function did) meant real, observed behavior: obligations climbed
        // steadily while health_board() stayed at 0 complete entries for
        // the entire time it took the queue to drain everything else
        // first. Reserves first means the first obligations to arrive
        // already have what they need to produce a real health factor.
        let scope_prices_id = account_id_from_pubkey(&Pubkey::from_str_const(XSTOCKS_SCOPE_PRICES_ACCOUNT));
        sub_queue.extend(s_reserve_id.iter().copied().chain([scope_prices_id]).map(|id| SubscriptionRequest {
            root: id,
            filter_weight: 0,
            depth: 0,
        }));
        sub_queue.extend(obligation_ids.iter().map(|&id| SubscriptionRequest {
            root: id,
            filter_weight: 0,
            depth: 0,
        }));

        let s_xstock_reserve_id: HashSet<AccountId> = XSTOCKS_RESERVES
            .iter()
            .map(|(s, _ticker)| account_id_from_pubkey(&Pubkey::from_str_const(s)))
            .collect();

        Self {
            m_obligation: HashMap::new(),
            m_reserve: HashMap::new(),
            s_reserve_id,
            s_xstock_reserve_id,
            scope_prices_id,
            o_scope_prices_body: None,
            sub_queue,
            _subscriptions: Vec::new(),
        }
    }

    /// Which real SPL token program a reserve's *liquidity* mint (not its
    /// cToken -- that's always classic SPL Token regardless, see
    /// `kamino::deposit_with_token_program`'s doc comment) is owned by.
    /// Only independently on-chain-verified for two reserves this session:
    /// TSLAx (Token-2022, confirmed via the real testperpv1 deposit test)
    /// and USDC (classic SPL, the canonical mint). For every other
    /// reserve, this infers from which const list it's in -- all 10
    /// [`XSTOCKS_RESERVES`] assumed Token-2022 (Backed Finance's published
    /// xStocks all share the same extension template across tickers), the
    /// 2 remaining [`XSTOCKS_DEBT_RESERVES`] (cbBTC, USDG) assumed classic
    /// SPL (ordinary, unwrapped tokens, unlike the xStocks side). Not
    /// independently confirmed on-chain for those 3 -- if a liquidation
    /// against one of them ever fails with a token-program mismatch, this
    /// assumption is the first thing to check.
    pub fn liquidity_token_program(&self, reserve_id: AccountId) -> Pubkey {
        if self.s_xstock_reserve_id.contains(&reserve_id) {
            super::tslax::TOKEN_2022_PROGRAM_ID
        } else {
            spl_token::ID
        }
    }

    /// How many known accounts (obligations + reserves) are still waiting
    /// to be subscribed to. `0` once [`Self::flush_subscriptions`] has
    /// fully drained the startup queue.
    pub fn pending_subscriptions(&self) -> usize {
        self.sub_queue.pending_count()
    }

    /// Drain up to `max_per_flush` queued subscriptions into one bounded
    /// `bulk_subscribe` call. Call once per slot (e.g. from
    /// `CommitHook::finish`, the same place every other `SubscriptionQueue`
    /// user in this codebase calls it) until [`Self::pending_subscriptions`]
    /// reaches `0` -- never all at once, see this module's own doc
    /// comment for why.
    pub fn flush_subscriptions(
        &mut self,
        g: &Graph,
        max_per_flush: usize,
    ) -> Result<usize, crate::err::CatscopeGuestError> {
        let n = self.sub_queue.flush(g, max_per_flush)?;
        Ok(n)
    }

    pub fn on_account(&mut self, header: &Header, body: &[u8]) {
        // Real, live-confirmed incident this exact check guards against
        // (`tslax::TslaxState`'s identical fix):
        // merely receiving an `on_account` push for a subscribed id is
        // NOT proof the account exists -- a subscription to an address
        // with nothing on it yet, or one that's since been closed, still
        // produces exactly one push (owner = System Program, empty/zeroed
        // body, `lamports == 0`). Require real, currently rent-exempt,
        // Kamino-owned data before trusting either parser's result, and
        // drop any tracked entry this update disproves.
        let is_real = header.owner == account_id_from_pubkey(&KAMINO_LENDING_PROGRAM_ID)
            && header.lamports > 0;

        if header.accountid == self.scope_prices_id {
            // Scope's OraclePrices account is owned by SCOPE_PROGRAM_ID,
            // not KAMINO_LENDING_PROGRAM_ID -- `is_real` above doesn't
            // apply here, needs its own check.
            let scope_is_real =
                header.owner == account_id_from_pubkey(&SCOPE_PROGRAM_ID) && header.lamports > 0;
            self.o_scope_prices_body = if scope_is_real { Some(body.to_vec()) } else { None };
            return;
        }

        if self.s_reserve_id.contains(&header.accountid) {
            if is_real {
                if let Some(r) = parse_kamino_reserve(body) {
                    self.m_reserve.insert(header.accountid, r);
                    return;
                }
            }
            self.m_reserve.remove(&header.accountid);
            return;
        }

        if is_real {
            if let Some(ob) = parse_kamino_obligation(body) {
                self.m_obligation.insert(header.accountid, ob);
                return;
            }
        }
        self.m_obligation.remove(&header.accountid);
    }

    /// How many of the known obligations currently have live, parsed data
    /// -- bounded by however many addresses `new` subscribed to (the
    /// generated list, or the fallback sample -- see both consts' doc
    /// comments), less than that until every subscription has both
    /// flushed and delivered its first real update.
    pub fn tracked_count(&self) -> usize {
        self.m_obligation.len()
    }

    pub fn obligations(&self) -> impl Iterator<Item = (&AccountId, &KaminoObligation)> {
        self.m_obligation.iter()
    }

    pub fn obligation(&self, id: AccountId) -> Option<&KaminoObligation> {
        self.m_obligation.get(&id)
    }

    /// Live-tracked state for one reserve (collateral- or debt-side, both
    /// live in the same `m_reserve` map -- see [`Self::on_account`]'s
    /// `s_reserve_id` check). `None` if `id` isn't a tracked reserve, or
    /// is but hasn't delivered a real update yet. Needed by callers that
    /// actually act on a [`Self::health_board`] entry (not just display
    /// it): building a real liquidation instruction needs each reserve's
    /// own account fields (mint, vaults, oracle accounts), not just the
    /// USD figures `health()` reduces them to.
    pub fn reserve(&self, id: AccountId) -> Option<&KaminoReserve> {
        self.m_reserve.get(&id)
    }

    /// [`KaminoReserve::live_price_usd`] for a given xStock ticker
    /// (`"TSLAx"`, etc.) instead of a reserve `AccountId` -- for callers
    /// (the arb-detection tracker in `state.rs`) that only have the
    /// ticker string, same reasoning as `ticker_to_id`'s own doc
    /// comment. `None` if the ticker isn't one of the 10 xStock
    /// reserves, or that reserve hasn't delivered live data yet.
    pub fn live_price_usd_by_ticker(&self, ticker: &str) -> Option<f64> {
        let (addr, _) = XSTOCKS_RESERVES.iter().find(|(_, t)| *t == ticker)?;
        let id = account_id_from_pubkey(&Pubkey::from_str_const(addr));
        Some(self.m_reserve.get(&id)?.live_price_usd(self.o_scope_prices_body.as_deref()))
    }

    /// Total number of xStock/debt reserves this watcher subscribes to
    /// (a fixed constant, [`XSTOCKS_RESERVES`] + [`XSTOCKS_DEBT_RESERVES`]
    /// = 13) -- for the dashboard's "N reserves monitored" stat.
    pub fn reserve_count() -> usize {
        XSTOCKS_RESERVES.len() + XSTOCKS_DEBT_RESERVES.len()
    }

    /// Computes [`ObligationHealth`] for one tracked obligation from its
    /// deposits/borrows and whichever referenced reserves have live data
    /// so far. `None` only if `id` isn't a currently-tracked obligation at
    /// all -- see [`ObligationHealth::complete`] for the "some reserves
    /// still missing" case.
    pub fn health(&self, id: AccountId) -> Option<ObligationHealth> {
        let ob = self.m_obligation.get(&id)?;
        let mut collateral_usd = 0.0;
        let mut debt_usd = 0.0;
        let mut complete = true;
        let mut best_collateral: Option<(AccountId, f64)> = None;
        let mut best_debt: Option<(AccountId, f64)> = None;
        // Priced via KaminoReserve::underlying_to_usd_live, not
        // underlying_to_usd -- real, live-diagnosed incident this fixes
        // (2026-09-25): a reserve's own cached price_usd is only as fresh
        // as the last time *anyone* sent a Kamino instruction touching
        // that specific reserve, which can lag Scope's ~30-40s refresh
        // cadence by minutes on a quiet reserve (SPYx: 364s stale cached
        // vs. 37s stale via Scope directly, same moment) -- so a quiet
        // reserve's obligations could sit well past real liquidation
        // eligibility with health() never noticing. Falls back to the
        // cached price automatically when o_scope_prices_body hasn't
        // arrived yet or a reserve's scope_price_chain_index is None --
        // see live_price_usd's own doc comment.
        let scope_body = self.o_scope_prices_body.as_deref();

        for c in &ob.deposits {
            match self.m_reserve.get(&c.deposit_reserve) {
                Some(r) => {
                    let underlying = r.ctokens_to_underlying(c.deposited_amount);
                    let usd = r.underlying_to_usd_live(underlying, scope_body);
                    collateral_usd += usd * r.liquidation_threshold_pct;
                    if best_collateral.is_none_or(|(_, best)| usd > best) {
                        best_collateral = Some((c.deposit_reserve, usd));
                    }
                }
                None => complete = false,
            }
        }
        for l in &ob.borrows {
            match self.m_reserve.get(&l.borrow_reserve) {
                Some(r) => {
                    let usd = r.underlying_to_usd_live(l.borrowed_amount as f64, scope_body);
                    debt_usd += usd;
                    if best_debt.is_none_or(|(_, best)| usd > best) {
                        best_debt = Some((l.borrow_reserve, usd));
                    }
                }
                None => complete = false,
            }
        }

        let health_factor = if debt_usd > 0.0 {
            collateral_usd / debt_usd
        } else {
            f64::INFINITY
        };
        Some(ObligationHealth {
            collateral_usd,
            debt_usd,
            health_factor,
            complete,
            collateral_ticker_id: best_collateral.map_or(255, |(id, _)| reserve_ticker_id(id)),
            debt_ticker_id: best_debt.map_or(255, |(id, _)| reserve_ticker_id(id)),
        })
    }

    /// Hard ceiling on `health_board()`'s output, in entries. This is not
    /// a display preference -- it's load-bearing. The outbound wire
    /// message carrying the board is one `catmsg` custom message, and
    /// `catmsg::BUFMAX` (4096 bytes, `github.com/noncepad/catmsg`,
    /// unconfigurable from this side) is a hard per-message ceiling on
    /// *any* message this bot ever sends. Real, observed incident this
    /// guards against, 2026-09-17: with the full ~6,953-obligation
    /// xStocks market (vs. the original 8-address sample, which never
    /// came close to any limit), the board grew past that ceiling within
    /// seconds of a real run -- `optimizer` logged `stdout error:
    /// extractCustom error: too big: 4824` and the whole bot process
    /// exited. True theoretical max at 34 bytes/entry (see message.rs's
    /// `HEALTH_BOARD_ENTRY_SIZE`) plus the 8-byte total_tracked header
    /// and 4-byte KeyValuePair framing is `(4096-4-8)/34 = 120` entries
    /// exactly -- 110 leaves real headroom (3748 bytes total, vs. the
    /// 4096 cap) rather than sitting right at that edge. Also just
    /// better product design regardless: a live "most at-risk" board
    /// with thousands of rows serves nobody; the dashboard's own
    /// pagination (20/page) is what actually makes 110 browsable.
    const HEALTH_BOARD_MAX_ENTRIES: usize = 110;

    /// [`Self::health`] for every currently-tracked obligation, ascending
    /// by health factor (most at-risk first) -- the sort order the
    /// hackathon plan's dashboard (`HACKATHON_PLAN.md` §5.1) wants.
    /// Skips entries [`ObligationHealth::complete`] marks incomplete
    /// rather than showing a misleadingly low/high partial number.
    /// Truncated to [`Self::HEALTH_BOARD_MAX_ENTRIES`] -- see that
    /// const's own doc comment, this is a hard wire-protocol requirement,
    /// not just a display choice.
    pub fn health_board(&self) -> Vec<(AccountId, ObligationHealth)> {
        let mut out: Vec<(AccountId, ObligationHealth)> = self
            .m_obligation
            .keys()
            .filter_map(|&id| self.health(id).filter(|h| h.complete).map(|h| (id, h)))
            .collect();
        out.sort_by(|a, b| a.1.health_factor.total_cmp(&b.1.health_factor));
        out.truncate(Self::HEALTH_BOARD_MAX_ENTRIES);
        out
    }

    /// Count of every *complete* tracked obligation (not just the
    /// top-[`Self::HEALTH_BOARD_MAX_ENTRIES`] shown on the board) in each
    /// of 4 health-factor tiers: `[0]` eligible now (`<= 1.0`), `[1]`
    /// at-risk (`1.0..1.5`), `[2]` watch (`1.5..3.0`), `[3]` safe
    /// (`>= 3.0`, including debt-free/infinite). Same tier boundaries the
    /// dashboard's existing "at risk" (`< 1.5`) coloring already uses for
    /// the first two tiers. Pays the same per-obligation `health()` cost
    /// `health_board()` does -- deliberately not derived from that
    /// function's own output, since that's capped at 100 entries and
    /// this needs the real total across every tracked obligation.
    pub fn risk_tier_counts(&self) -> [u64; 4] {
        let mut counts = [0u64; 4];
        for &id in self.m_obligation.keys() {
            let Some(h) = self.health(id).filter(|h| h.complete) else {
                continue;
            };
            let tier = if h.health_factor <= 1.0 {
                0
            } else if h.health_factor < 1.5 {
                1
            } else if h.health_factor < 3.0 {
                2
            } else {
                3
            };
            counts[tier] += 1;
        }
        counts
    }
}
