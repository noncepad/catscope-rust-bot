use solana_sdk::{clock::Slot, transaction::TransactionError};

use crate::{
    catscope::witbot::shooter::{Header, Tokenaccountv1},
    err::CatscopeGuestError,
    graph::{AccountId, Graph},
    trader::pricegraph::TradeRouter,
    txview::CatscopeInstructionRead,
};

/// What kind of venue `on_account` just updated, plus whatever
/// price/rate value that update carries -- lets a caller (a strategy's
/// own `StateHelper`) react to a specific account update without
/// needing to separately re-derive "did anything I care about just
/// change" itself. Every variant other than [`StateChange::Blank`] is
/// only ever constructed when it carries a real, usable number (or, for
/// `Lending`, at least one of its three) -- an account that was
/// recognized and processed but has nothing worth a caller's attention
/// reports [`StateChange::Blank`] instead, never a category variant with
/// every field empty.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StateChange {
    /// Nothing meaningful to report. Returned (rather than `on_account`
    /// itself returning `None`) when the account *was* recognized and
    /// processed by this dex, but there's no real update worth a
    /// caller's attention -- e.g. a swap-pool account whose price-bearing
    /// fields only change via `on_token`, not this update; a shared/
    /// global account (Sanctum's pool-wide list, Pump.fun/PumpSwap's
    /// global-config) with no single pool to attribute a price to; or an
    /// invalid/not-yet-priced pool. Distinct from `on_account` returning
    /// `None`, which means this dex didn't recognize the account at all
    /// (see `DexState::on_account`'s fan-out, which relies on that
    /// distinction to keep checking other dexes).
    Blank,
    /// A swap-pool price update (Orca/Raydium/PumpSwap/Pump.fun/
    /// Marinade/`spl_stake_pool`) -- only ever returned when a real,
    /// fresh price is available *and* both mints are known; anything
    /// less (no price yet, or a shared/global account with no single
    /// pool) is [`StateChange::Blank`] instead, never a half-populated
    /// `Spot`. `token_a`/`token_b` identify *which pool* by its two
    /// mints (not the pool account's own address, which callers rarely
    /// have memorized). `price` is the pool's own raw exchange rate
    /// (protocol-native units, e.g. token_b per token_a for a
    /// constant-product/CLMM pool) -- **not** normalized to USD; that
    /// needs external anchor-price context (see `pricegraph::TradeRouter`)
    /// this trait's implementors don't have.
    Spot {
        token_a: AccountId,
        token_b: AccountId,
        price: f64,
    },
    /// A lending reserve/bank account update (Solend/Kamino/Marginfi/
    /// Drift-spot) carrying at least one real number -- only ever
    /// returned when `borrow_apy`/`supply_apy`/`price_usd` aren't *all*
    /// `None`; an obligation/position-account update (nothing rate/
    /// price-bearing to report at all) or an unparseable account is
    /// [`StateChange::Blank`] instead. Each field can still
    /// independently be `None` even when returned -- unlike `Spot`,
    /// these three are only ever available independently of each other
    /// (e.g. marginfi's *oracle* account update reports a new
    /// `price_usd` alone, both APY fields `None`, since a price tick
    /// doesn't change either share value; Drift never reports
    /// `supply_apy` at all, no estimator exists for it). `borrow_apy`/
    /// `supply_apy` are fractions per year (e.g. `0.042` = 4.2%);
    /// `price_usd` is the underlying asset's oracle price.
    Lending {
        account_id: AccountId,
        borrow_apy: Option<f64>,
        supply_apy: Option<f64>,
        price_usd: Option<f64>,
    },
    /// A perpetual-futures market price update (Phoenix) -- only ever
    /// returned when a real mark price is attributable to *one*
    /// specific market *and* both mints are known, same discipline as
    /// `Spot`. `token_a` is the market's underlying/base mint (e.g. SOL);
    /// `token_b` is the quote/collateral mint (Phoenix's PhUSD, not
    /// USDC -- see `dex::phoenix::mod`'s "Collateral is not USDC" module
    /// doc). Not currently constructed by any implementor: every Phoenix
    /// account type this bot tracks is either shared across every
    /// market (`PerpAssetMap`), the trader's own position account, or
    /// global config -- none is a single market's own address, so
    /// there's no sound way to attribute a price to "this account" for
    /// Phoenix at all (let alone its mints); see
    /// `dex::phoenix::PhoenixState::on_account`'s doc comment. Kept for
    /// a future protocol/account shape that can genuinely satisfy it
    /// (e.g. a per-market price account).
    Perp {
        token_a: AccountId,
        token_b: AccountId,
        mark_price_usd: f64,
    },
}

pub trait Updater {
    fn on_account(&mut self, header: &Header, body: &[u8]) -> Option<StateChange>;
    fn on_token(&mut self, ta: &Tokenaccountv1) -> bool;
    fn on_tx(&mut self, ix: &CatscopeInstructionRead<'_>, r: &Result<Slot, TransactionError>);
    fn batch_router(&mut self, router: &mut TradeRouter);
    /// `max_per_flush` bounds how many queued subscription requests a
    /// single call may drain into one `bulk_subscribe` -- implementors
    /// with nothing to defer (most of them) just ignore it and return
    /// `Ok(())`. See `graph::SubscriptionQueue::flush`'s doc comment for
    /// why an unbounded batch here is unsafe (a confirmed blocking host
    /// call, and the real cause of this session's `stdio timeout` hangs).
    fn flush_pool(&mut self, graph: &Graph, max_per_flush: usize)
        -> Result<(), CatscopeGuestError>;

    /// Incrementally refresh just the router edge(s) touched by an
    /// `on_account` update for `account_id`, if it's one of this dex's
    /// pool accounts -- called right after `on_account` so live per-pool
    /// state (~400ms cadence) reaches `TradeRouter` without waiting for
    /// the periodic ~12s full `batch_router` resync. Default no-op for
    /// dexes that never feed router edges (Kamino, Marginfi, Solend,
    /// Drift, Phoenix), matching `batch_router`'s existing no-op
    /// convention for those.
    fn refresh_account_router(&mut self, _account_id: AccountId, _router: &mut TradeRouter) {}

    /// Same as [`refresh_account_router`](Self::refresh_account_router),
    /// but for a token/vault-balance update (`on_token`) -- resolves the
    /// pool that owns this token account via whatever mapping `on_token`
    /// already uses internally, then re-derives just that pool's edges.
    fn refresh_token_router(&mut self, _ta_id: AccountId, _router: &mut TradeRouter) {}
}
