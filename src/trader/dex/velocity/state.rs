//! `VelocityState` -- live subscription + parsed-market storage for a
//! small, hand-curated set of Drift/Velocity perp markets. Unlike
//! `dex::phoenix::PhoenixState`, no multi-hop account discovery is
//! needed: each tracked market's `PerpMarket` account address is
//! directly derivable from its `market_index` via a fixed PDA formula
//! (`["perp_market", market_index_u16_le]` against `DRIFT_PROGRAM_ID`),
//! confirmed live this session (see `accounts.rs`'s module doc).
//!
//! Deliberately **not** wired into `DexState`/`Updater` -- only
//! `brain::perpfundingv1` constructs this, mirroring how
//! `dex::phoenix::PhoenixState`'s authority-bearing instance is
//! independent of the read-only one `DexState` owns.

use super::accounts::{parse_perp_market, VelocityPerpMarketView};
use crate::{
    catscope::witbot::shooter::Header,
    err::CatscopeGuestError,
    graph::{AccountId, Graph, Subscription, SubscriptionQueue, SubscriptionRequest},
    trader::dex::drift::{self, DRIFT_PROGRAM_ID},
    util::{account_id_from_pubkey, resolve_symbol_mint},
};
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;

/// `(symbol, market_index)` for every Velocity market this bot tracks --
/// hand-curated, matching exactly the Phoenix symbols confirmed (live,
/// this session -- scanned `market_index` 0-34, decoded each real
/// account's `name`) to also have a real Velocity market: SOL, BTC, ETH,
/// XRP, BNB, DOGE, SUI. Phoenix also tracks HYPE/SKR/AAVE, but none of
/// those were found among Velocity's first 35 `market_index` slots (not
/// chased further -- 7 confirmed real overlapping majors is enough for a
/// working v1; a genuinely missing market on one side just never
/// produces a `FundingEdge` for that symbol, which is correct, not a bug).
pub const TRACKED_MARKETS: &[(&str, u16)] =
    &[("SOL", 0), ("BTC", 1), ("ETH", 2), ("XRP", 13), ("BNB", 8), ("DOGE", 7), ("SUI", 9)];

fn perp_market_pda(market_index: u16) -> Pubkey {
    Pubkey::find_program_address(&[b"perp_market", &market_index.to_le_bytes()], &DRIFT_PROGRAM_ID).0
}

#[derive(Debug)]
pub struct VelocityState {
    program_id: AccountId,
    /// Subscribed `PerpMarket` account -> its last successful parse.
    /// `None` until the first update arrives for that account.
    m_market: HashMap<AccountId, Option<VelocityPerpMarketView>>,
    /// Same keys as `m_market` -- each market's `base_mint`, resolved
    /// once here (since `TRACKED_MARKETS`' symbols are known up front,
    /// unlike the view itself, which only exists after a first
    /// successful parse) and copied onto the view in `on_account`. See
    /// `VelocityPerpMarketView::base_mint`'s doc comment.
    m_base_mint: HashMap<AccountId, Option<AccountId>>,
    subscriptions: Vec<Subscription>,
    o_authority_pk: Option<Pubkey>,
    o_user_id: Option<AccountId>,
    /// `true` only once a real update for this bot's own Drift `User`
    /// account has been parsed -- means the account actually exists
    /// on-chain (`initialize_user` already succeeded), not just "we know
    /// its address and subscribed." Reliable because subscriptions are
    /// push-based: an account that doesn't exist yet simply never
    /// produces an `on_account` update. Mirrors `PhoenixState::
    /// trader_registered`'s exact reasoning.
    user_registered: bool,
    /// Last successfully parsed `User` account -- real position data
    /// (`perp_positions`), not just the registration bool above. Needed
    /// so `perpfundingv1`'s open-gate/close-decision can read actual
    /// held size/direction instead of separate bookkeeping.
    o_drift_user: Option<drift::DriftUser>,
}

impl VelocityState {
    pub fn new(g: &Graph) -> Result<Self, CatscopeGuestError> {
        let mut m_market = HashMap::with_capacity(TRACKED_MARKETS.len());
        let mut m_base_mint = HashMap::with_capacity(TRACKED_MARKETS.len());
        let mut l_req = Vec::with_capacity(TRACKED_MARKETS.len());
        for &(symbol, market_index) in TRACKED_MARKETS {
            let pda = perp_market_pda(market_index);
            let account_id = account_id_from_pubkey(&pda);
            l_req.push(SubscriptionRequest {
                root: account_id,
                filter_weight: 0,
                depth: 1,
            });
            m_market.insert(account_id, None);
            m_base_mint.insert(account_id, resolve_symbol_mint(symbol));
        }
        let subscriptions = SubscriptionQueue::subscribe_now(g, l_req)?;
        Ok(Self {
            program_id: account_id_from_pubkey(&DRIFT_PROGRAM_ID),
            m_market,
            m_base_mint,
            subscriptions,
            o_authority_pk: None,
            o_user_id: None,
            user_registered: false,
            o_drift_user: None,
        })
    }

    pub fn program_id(&self) -> &AccountId {
        &self.program_id
    }

    /// Register the wallet's authority pubkey (arrives via a stdin
    /// `Wallet` message) -- derives and subscribes to this bot's own
    /// Drift `User` sub-account 0 PDA. Idempotent, structurally mirrors
    /// `PhoenixState::set_authority` exactly.
    pub fn set_authority(&mut self, authority: Pubkey, g: &Graph) -> Result<(), CatscopeGuestError> {
        if self.o_authority_pk == Some(authority) {
            return Ok(());
        }
        let user_pk = drift::user_pda(&authority, 0);
        let user_id = account_id_from_pubkey(&user_pk);
        let sub = g.subscribe(SubscriptionRequest { root: user_id, filter_weight: 0, depth: 1 })?;
        self.subscriptions.push(sub);
        self.o_authority_pk = Some(authority);
        self.o_user_id = Some(user_id);
        Ok(())
    }

    /// See [`Self::user_registered`]'s field doc.
    pub fn user_registered(&self) -> bool {
        self.user_registered
    }

    /// Last successfully parsed `User` account, if any real update has
    /// arrived yet -- see [`Self::o_drift_user`]'s field doc.
    pub fn drift_user(&self) -> Option<&drift::DriftUser> {
        self.o_drift_user.as_ref()
    }

    pub fn on_account(&mut self, header: &Header, body: &[u8]) {
        if let Some(slot) = self.m_market.get_mut(&header.accountid) {
            if let Some(mut parsed) = parse_perp_market(body) {
                parsed.base_mint = self.m_base_mint.get(&header.accountid).copied().flatten();
                *slot = Some(parsed);
            }
            return;
        }
        if Some(header.accountid) == self.o_user_id {
            if let Some(u) = drift::parse_user(body) {
                self.user_registered = true;
                self.o_drift_user = Some(u);
            }
        }
    }

    /// Every tracked market with at least one successful parse so far.
    /// Each market carries its own real `name` (e.g. `"SOL-PERP"`), so
    /// no separate symbol needs to travel alongside it --
    /// `PerpRouter::observe_velocity` reads it directly via `name_str()`.
    pub fn markets(&self) -> impl Iterator<Item = &VelocityPerpMarketView> {
        self.m_market.values().filter_map(|m| m.as_ref())
    }

    /// How many of `TRACKED_MARKETS` have delivered at least one parse
    /// so far -- diagnostic only.
    pub fn ready_count(&self) -> usize {
        self.m_market.values().filter(|m| m.is_some()).count()
    }
}
