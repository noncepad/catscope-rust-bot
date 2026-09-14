use solana_sdk::clock::Slot;

use crate::{
    catscope::witbot::shooter::{Header, Tokenaccountv1},
    err::CatscopeGuestError,
    graph::{AccountId, Graph},
    trader::pricegraph::TradeRouter,
    txview::CatscopeInstructionRead,
};

pub trait Updater {
    fn on_account(&mut self, header: &Header, body: &[u8]);
    fn on_token(&mut self, ta: &Tokenaccountv1) -> bool;
    fn on_tx(&mut self, ix: &CatscopeInstructionRead<'_>, slot: &Slot);
    fn batch_router(&mut self, router: &mut TradeRouter);
    /// `max_per_flush` bounds how many queued subscription requests a
    /// single call may drain into one `bulk_subscribe` -- implementors
    /// with nothing to defer (most of them) just ignore it and return
    /// `Ok(())`. See `graph::SubscriptionQueue::flush`'s doc comment for
    /// why an unbounded batch here is unsafe (a confirmed blocking host
    /// call, and the real cause of this session's `stdio timeout` hangs).
    fn flush_pool(&mut self, graph: &Graph, max_per_flush: usize) -> Result<(), CatscopeGuestError>;

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
