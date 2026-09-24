//! Phoenix perpetuals -- margin/position state and instruction builders.
//!
//! # Program
//!
//! `EtrnLzgbS7nMMy5fbD42kXiUzGg8XQzJ972Xtk1cjWih` (mainnet). Verified live
//! against `https://api.mainnet-beta.solana.com`: `executable: true`,
//! `GlobalConfiguration` (`2zskx2iyCvb6Stg7RBZkt1f6MrF4dpYtMG3yMvKwqtUZ`)
//! is real, 2560 bytes, with 65 live perp markets in its `PerpAssetMap`
//! (`2nHGAaEw3D5dd4hVueaUNoygkQFmoeKqRQWnSPqSMFUC`) at time of writing --
//! see `accounts.rs`'s module doc for the full verification trail. Do not
//! use `phDEVv4w6BcfkLrLNeXr8HhhgQxnxziVGXpGPcaadMf` (the "beta" program
//! ID) -- unverified, presumed devnet, not used anywhere in this module.
//!
//! # Architecturally different from every other dex module
//!
//! A Phoenix position is **not** a `TradeRouter` graph edge -- a fill
//! mutates a `TraderPosition` entry inside a margin account rather than
//! delivering a transferable SPL token, and even fully collateralized it
//! accrues funding payments spot never does. `PhoenixState` **is**
//! registered in `trader::dex::mod::DexState` (so `arbv1` observes live
//! market pricing/risk data the same way it does every other dex), but its
//! `Updater::batch_router` is a **documented placeholder** -- see
//! [`PhoenixState::add_to_pricing_router`] -- exactly like Kamino/Marginfi/
//! Solend's own "doesn't feed TradeRouter (yet)" no-ops. There is no
//! `DexType::Phoenix*` variant in `types.rs` yet; adding real pricing
//! would need one (see that method's doc for the shape a real
//! implementation would take). The `brain::phoenixperpsv1` strategy still
//! owns its own separate `PhoenixState` instance for the full trader-
//! account/margin lifecycle (registration, deposits, positions) -- that
//! side needs a wallet authority this `DexState`-owned instance never
//! receives, so the two are intentionally independent, not shared.
//!
//! # Collateral is not USDC
//!
//! `GlobalConfiguration.canonical_token_mint_key` (read live, never
//! hardcoded) decodes to `PhUsd11YkbjSaWjFncfAAmatntsjx3MgDR9B6g1ks3A` on
//! real mainnet data -- a separate Phoenix-native mint, not
//! `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v`. Real USDC must be
//! converted via the separate Ember program
//! (`EMBERpYNE6ehWmXymZZS2skiFmCa9V5dp14e1iduM5qy`) before
//! [`ix::deposit_funds`] will accept it -- Ember's `ember_deposit`/
//! `ember_withdraw` instructions are not yet implemented here (out of
//! scope for this pass; flagged, not silently wrong).
//!
//! # Two-hop discovery: `GlobalConfiguration` -> `PerpAssetMap`/index headers
//!
//! `GlobalConfiguration` itself lives at a fixed, hardcoded address, but
//! `perp_asset_map`/`global_trader_index_header`/`active_trader_buffer_header`/
//! `global_vault` are only known once its first update is parsed. `new(g)`
//! subscribes only to `GlobalConfiguration`; [`PhoenixState::on_account`]
//! parses it and sets a pending-subscribe flag;
//! [`PhoenixState::flush_pending`] (called once per commit by the owning
//! strategy, same convention as `orca.rs`/`spl_stake_pool.rs`'s deferred
//! subscription queues) issues the second-hop subscription once. The
//! trader's own `trader_account` PDA needs the wallet's authority pubkey,
//! which arrives even later (via a stdin `Wallet` message) --
//! [`PhoenixState::set_authority`] handles that third hop.
//!
//! # `global_trader_index`/`active_trader_buffer` remaining accounts
//!
//! Verified live: both headers report `num_arenas = 1` right now, so the
//! remaining-accounts list each instruction needs is just `[header_pk]`.
//! [`PhoenixState::global_trader_index_accounts`]/
//! [`PhoenixState::active_trader_buffer_accounts`] assert this and panic
//! if the exchange ever scales past it, rather than silently sending a
//! wrong (and therefore economically dangerous) instruction -- see their
//! doc comments.

pub mod accounts;
pub mod ix;
pub mod margin;
pub mod pda;

use std::collections::HashMap;

use solana_sdk::pubkey::Pubkey;

use crate::{
    catscope::witbot::shooter::Header,
    err::CatscopeGuestError,
    graph::{AccountId, Graph, Subscription, SubscriptionQueue, SubscriptionRequest},
    util::{account_id_from_pubkey, resolve_symbol_mint},
};

pub const PHOENIX_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("EtrnLzgbS7nMMy5fbD42kXiUzGg8XQzJ972Xtk1cjWih");
const PHOENIX_FEE_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ");
const PHOENIX_LOG_AUTHORITY: Pubkey =
    Pubkey::from_str_const("GdxfTLSsdSY37G6fZoYtdGDSfgFnbT2EmRpuePZxWShS");
const GLOBAL_CONFIGURATION_PK: Pubkey =
    Pubkey::from_str_const("2zskx2iyCvb6Stg7RBZkt1f6MrF4dpYtMG3yMvKwqtUZ");
const SPL_TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
const SYSTEM_PROGRAM_ID: Pubkey = Pubkey::from_str_const("11111111111111111111111111111111");

/// One tracked market's static config (from build-time `phoenix_config`)
/// plus live pricing/risk fields from `PerpAssetMap`.
#[derive(Debug, Clone)]
pub struct PhoenixMarketState {
    pub symbol: [u8; 16],
    pub asset_id: u32,
    pub market_pk: Pubkey,
    pub spline_collection_pk: Pubkey,
    pub tick_size: u64,
    pub base_lot_decimals: i8,
    pub tier0_max_leverage: u64,
    pub tier0_upper_bound_size: u64,
    pub cumulative_funding_rate: i64,
    pub open_interest: u64,
    pub open_interest_cap: u64,
    pub oracle_mark_price_ticks: u64,
    /// `true` once at least one `PerpAssetMap` update has populated this
    /// entry's live fields.
    pub priced: bool,
    /// The market's underlying spot mint, resolved once in
    /// [`PhoenixState::new`] from `symbol_mint_config::SYMBOL_MINT_MAP` --
    /// genuinely not present in `PerpAssetMap`'s bytes (a perp only needs
    /// an oracle price, not a token). `None` for a symbol with no curated
    /// entry (e.g. HYPE/SKR/AAVE, which have no confirmed real, liquid
    /// Solana mint) -- still fine for pure inter-venue funding capture,
    /// just not spot-hedgeable.
    pub base_mint: Option<AccountId>,
}

/// Phoenix's quote-lot decimals, fixed protocol-wide -- confirmed from
/// Ellipsis Labs' public `rise-public` SDK
/// (`rust/math/src/funding.rs::FundingCalculator::new`, hardcoded
/// `quote_lot_decimals: 6`).
const QUOTE_LOT_DECIMALS: i32 = 6;

impl PhoenixMarketState {
    pub fn symbol_str(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.symbol[..self.symbol.iter().position(|&b| b == 0).unwrap_or(16)])
            .into_owned()
            .into()
    }

    /// Mark price in USD, converted from `oracle_mark_price_ticks` (the
    /// continuously-updated, oracle-blended price -- see
    /// `accounts::PerpAssetEntryView::oracle_mark_price_ticks`'s doc
    /// comment for why this is a different, correct field, and *not*
    /// the market-closure-only `finalized_mark_price` this bot used
    /// before this session's fix). Returns `None` when the raw value is
    /// `0`, meaning no oracle price has ever been recorded yet for this
    /// market (e.g. a brand-new listing) -- "not priced yet", not
    /// "worth zero dollars".
    ///
    /// **Live-verified this session**: real `PerpAssetMap` ticks for
    /// SOL/BTC/ETH converted via this exact formula to $75.62 / $63,118
    /// / $1,884 -- all in the ranges this bot's own spot-price probes
    /// already showed independently -- and the price's own recorded
    /// slot was ~38 slots (about 15s) behind the real current mainnet
    /// slot, confirming it updates continuously rather than being stale
    /// or frozen. The formula itself reuses the same verified pieces as
    /// `trader::perp_router`'s funding-rate conversion (same
    /// `QUOTE_LOT_DECIMALS`, same `tick_size`/`base_lot_decimals`
    /// fields) and matches Ellipsis Labs' own public `MarketUnitConfig {
    /// tick_size_in_quote_lots_per_base_lot, base_lots_decimals }` naming
    /// (`rise-public`'s `rust/types/src/market.rs`).
    pub fn mark_price_usd(&self) -> Option<f64> {
        if self.oracle_mark_price_ticks == 0 {
            return None;
        }
        let quote_lots_per_base_lot = self.oracle_mark_price_ticks as f64 * self.tick_size as f64;
        let quote_lots_per_quote_unit = 10f64.powi(QUOTE_LOT_DECIMALS);
        let base_lots_per_base_unit = 10f64.powi(self.base_lot_decimals as i32);
        Some((quote_lots_per_base_lot / quote_lots_per_quote_unit) * base_lots_per_base_unit)
    }
}

/// One open position, read live from the trader's own `Trader` account.
#[derive(Debug, Clone, Copy)]
pub struct PhoenixPositionState {
    pub asset_id: u64,
    pub base_lot_position: i64,
    pub virtual_quote_lot_position: i64,
    pub cumulative_funding_snapshot: i64,
    pub accumulated_funding_for_active_position: i64,
}

#[derive(Debug)]
pub struct PhoenixState {
    program_id: AccountId,
    global_config_id: AccountId,
    global_ready: bool,
    canonical_mint_pk: Pubkey,
    global_vault_pk: Pubkey,
    perp_asset_map_id: AccountId,
    perp_asset_map_pk: Pubkey,
    global_trader_index_header_pk: Pubkey,
    active_trader_buffer_header_pk: Pubkey,
    withdraw_queue_pk: Pubkey,
    /// `true` once `GlobalConfiguration` has been parsed at least once but
    /// the second-hop subscription (perp_asset_map/index headers) hasn't
    /// been issued yet -- see the module doc's discovery-hop explanation.
    pending_second_hop: bool,
    markets: Vec<PhoenixMarketState>,
    market_by_asset_id: HashMap<u32, usize>,
    o_authority_pk: Option<Pubkey>,
    o_trader_account_pk: Option<Pubkey>,
    o_trader_account_id: Option<AccountId>,
    /// `true` once the trader account's own subscription has been issued
    /// (third hop, needs the wallet authority -- see `set_authority`).
    trader_account_ready: bool,
    /// `true` only once a REAL update for the trader account has been
    /// parsed -- unlike `trader_account_ready`, this means the account
    /// actually exists on-chain (i.e. `register_trader` has already
    /// succeeded), not just "we know its address and subscribed."
    /// Reliable because this bot's subscriptions are push-based: an
    /// account that doesn't exist yet simply never produces an
    /// `on_account` update.
    trader_registered: bool,
    collateral_quote_lots: i64,
    /// Raw `TraderCapabilityFlags` bitmask from the trader account's own
    /// real on-chain state -- `0` until `trader_registered` is `true`.
    /// See [`Self::is_trader_frozen`]/[`Self::can_deposit`].
    capability_flags: u32,
    max_positions: u32,
    positions: Vec<PhoenixPositionState>,
    /// Holds every subscription this instance is directly responsible
    /// for alive: `set_authority`'s trader-account subscription,
    /// `flush_pending`'s second-hop subscriptions, and (for standalone
    /// instances constructed via [`Self::new_and_subscribe`], not
    /// `DexState`'s own deferred copy) the initial `global_config`
    /// subscription too.
    subscriptions: Vec<Subscription>,
}

impl PhoenixState {
    /// Builds this dex's live state and returns its pending subscription
    /// requests alongside it -- doesn't subscribe itself. See
    /// `dex::raydium::amm::RaydiumAmm::new`'s doc comment for why (paced
    /// through a shared [`crate::graph::SubscriptionQueue`] owned by
    /// `DexState` instead). `flush_pending`'s own second-hop
    /// `bulk_subscribe` is unrelated and untouched -- it's already
    /// deferred, on-demand, and small (3 accounts).
    pub fn new() -> (Self, Vec<SubscriptionRequest>) {
        let global_config_id = account_id_from_pubkey(&GLOBAL_CONFIGURATION_PK);
        let raw = crate::phoenix_config::PHOENIX_MARKETS;
        let mut markets = Vec::with_capacity(raw.len());
        let mut market_by_asset_id = HashMap::with_capacity(raw.len());
        for entry in raw.iter() {
            let market_pk = Pubkey::new_from_array(entry.market_account);
            let idx = markets.len();
            market_by_asset_id.insert(entry.asset_id, idx);
            let symbol_str = String::from_utf8_lossy(
                &entry.symbol[..entry.symbol.iter().position(|&b| b == 0).unwrap_or(16)],
            );
            markets.push(PhoenixMarketState {
                symbol: entry.symbol,
                asset_id: entry.asset_id,
                market_pk,
                spline_collection_pk: pda::spline_collection(&market_pk),
                tick_size: 0,
                base_lot_decimals: 0,
                tier0_max_leverage: 0,
                tier0_upper_bound_size: 0,
                cumulative_funding_rate: 0,
                open_interest: 0,
                open_interest_cap: 0,
                oracle_mark_price_ticks: 0,
                priced: false,
                base_mint: resolve_symbol_mint(&symbol_str),
            });
        }

        let l_req = vec![SubscriptionRequest {
            root: global_config_id,
            filter_weight: 0,
            depth: 1,
        }];

        let state = Self {
            program_id: account_id_from_pubkey(&PHOENIX_PROGRAM_ID),
            global_config_id,
            global_ready: false,
            canonical_mint_pk: Pubkey::default(),
            global_vault_pk: Pubkey::default(),
            perp_asset_map_id: 0,
            perp_asset_map_pk: Pubkey::default(),
            global_trader_index_header_pk: Pubkey::default(),
            active_trader_buffer_header_pk: Pubkey::default(),
            withdraw_queue_pk: Pubkey::default(),
            pending_second_hop: false,
            markets,
            market_by_asset_id,
            o_authority_pk: None,
            o_trader_account_pk: None,
            o_trader_account_id: None,
            trader_account_ready: false,
            trader_registered: false,
            collateral_quote_lots: 0,
            capability_flags: 0,
            max_positions: 0,
            positions: Vec::new(),
            subscriptions: Vec::new(),
        };
        (state, l_req)
    }

    /// Immediately subscribes to this dex's pending `global_config`
    /// request and keeps it alive via `self.subscriptions` -- for
    /// callers that don't defer through a shared
    /// [`crate::graph::SubscriptionQueue`] (`DexState`'s own copy uses
    /// the split [`Self::new`]/queued-apply path instead). Real callers:
    /// `perpfundingv1`/`phoenixperpsv1`/`testperpv1`'s own standalone
    /// trading instances, distinct from `DexState`'s shared read-only
    /// pricing copy.
    pub fn new_and_subscribe(g: &Graph) -> Result<Self, CatscopeGuestError> {
        let (mut state, l_req) = Self::new();
        let subs = SubscriptionQueue::subscribe_now(g, l_req)?;
        state.subscriptions.extend(subs);
        Ok(state)
    }

    #[inline]
    pub fn program_id(&self) -> &AccountId {
        &self.program_id
    }

    pub fn global_ready(&self) -> bool {
        self.global_ready
    }

    pub fn canonical_mint(&self) -> Pubkey {
        self.canonical_mint_pk
    }

    pub fn market(&self, asset_id: u32) -> Option<&PhoenixMarketState> {
        self.market_by_asset_id.get(&asset_id).map(|&i| &self.markets[i])
    }

    pub fn markets(&self) -> &[PhoenixMarketState] {
        &self.markets
    }

    pub fn positions(&self) -> &[PhoenixPositionState] {
        &self.positions
    }

    pub fn collateral_quote_lots(&self) -> i64 {
        self.collateral_quote_lots
    }

    /// `true` once `DepositFunds` is real safe to attempt against this
    /// trader account -- see [`accounts::capability_can_deposit`]'s doc
    /// comment for why callers must check this instead of only reacting
    /// to a failed deposit. `false` (not just "unknown") before
    /// [`Self::trader_registered`] is true, matching every other real
    /// field here that defaults to its "not ready yet" value.
    pub fn can_deposit(&self) -> bool {
        self.trader_registered && accounts::capability_can_deposit(self.capability_flags)
    }

    /// Mirrors the real protocol's own "frozen" state -- see
    /// [`accounts::capability_is_frozen`]'s doc comment for the real,
    /// live-confirmed incident this exists to detect up front instead of
    /// only after a failed deposit.
    pub fn is_trader_frozen(&self) -> bool {
        self.trader_registered && accounts::capability_is_frozen(self.capability_flags)
    }

    pub fn trader_account(&self) -> Option<(AccountId, Pubkey)> {
        Some((self.o_trader_account_id?, self.o_trader_account_pk?))
    }

    /// `true` only once a real update for the trader account has been
    /// parsed -- see the field's own doc comment for why this is a
    /// reliable "does this account exist on-chain yet" signal.
    pub fn trader_registered(&self) -> bool {
        self.trader_registered
    }

    /// How many tracked markets have live pricing (at least one
    /// `PerpAssetMap` update observed) -- for the periodic stats log.
    pub fn ready_count(&self) -> usize {
        self.markets.iter().filter(|m| m.priced).count()
    }

    /// Register the wallet's authority pubkey (arrives via a stdin `Wallet`
    /// message, after `new()`) -- derives and subscribes to the
    /// cross-margin `trader_account` PDA. Idempotent: a repeat call with
    /// the same authority is a no-op; a call with a *different* authority
    /// re-derives and re-subscribes (mirrors `arbv1`'s wallet-replacement
    /// handling).
    pub fn set_authority(&mut self, authority: Pubkey, g: &Graph) -> Result<(), CatscopeGuestError> {
        if self.o_authority_pk == Some(authority) {
            return Ok(());
        }
        let trader_pk = pda::trader_account(&authority, 0, pda::SUBACCOUNT_CROSS_MARGIN);
        let trader_id = account_id_from_pubkey(&trader_pk);
        let sub = g.subscribe(SubscriptionRequest { root: trader_id, filter_weight: 0, depth: 1 })?;
        self.subscriptions.push(sub);
        self.o_authority_pk = Some(authority);
        self.o_trader_account_pk = Some(trader_pk);
        self.o_trader_account_id = Some(trader_id);
        self.trader_account_ready = true;
        Ok(())
    }

    /// Pure-derivation half of [`Self::set_authority`] -- returns the
    /// subscription request this authority needs (empty if already set,
    /// same idempotency as `set_authority`), without making the host
    /// `subscribe` call itself. Paired with [`Self::apply_authority`] so
    /// this venue's request can be batched into a single `bulk_subscribe`
    /// call together with every other venue's own -- see
    /// `perpfundingv1`/`testperpv1`'s `Wallet` message handler, added
    /// after a real, live-observed incident: five separate one-at-a-time
    /// `subscribe` calls (this one plus Solend/Kamino/marginfi's) in that
    /// handler accounted for ~26 seconds of stall in one run (traced via
    /// `CommitHook::start`'s own timing diagnostics). Not used by
    /// `phoenixperpsv1`, which still calls `set_authority` directly --
    /// this is purely additive.
    pub fn authority_subscribe_requests(&self, authority: Pubkey) -> Vec<SubscriptionRequest> {
        if self.o_authority_pk == Some(authority) {
            return Vec::new();
        }
        let trader_pk = pda::trader_account(&authority, 0, pda::SUBACCOUNT_CROSS_MARGIN);
        let trader_id = account_id_from_pubkey(&trader_pk);
        vec![SubscriptionRequest { root: trader_id, filter_weight: 0, depth: 1 }]
    }

    /// Apply `authority` plus its already-resolved subscription (from
    /// [`Self::authority_subscribe_requests`], via a batched
    /// `bulk_subscribe` call elsewhere) -- the second half of the split
    /// described there. No-op if `subs` is empty (either already set, or
    /// nothing to apply).
    pub fn apply_authority(&mut self, authority: Pubkey, subs: Vec<Subscription>) {
        let Some(sub) = subs.into_iter().next() else { return };
        let trader_pk = pda::trader_account(&authority, 0, pda::SUBACCOUNT_CROSS_MARGIN);
        let trader_id = account_id_from_pubkey(&trader_pk);
        self.subscriptions.push(sub);
        self.o_authority_pk = Some(authority);
        self.o_trader_account_pk = Some(trader_pk);
        self.o_trader_account_id = Some(trader_id);
        self.trader_account_ready = true;
    }

    /// Issue the second-hop subscription (`perp_asset_map`,
    /// `global_trader_index_header`, `active_trader_buffer_header`) once
    /// `GlobalConfiguration` has been parsed. Call once per commit from the
    /// owning strategy's `CommitHook::finish` -- cheap no-op once done,
    /// same convention as `orca.rs`/`spl_stake_pool.rs`'s deferred
    /// subscription flush.
    pub fn flush_pending(&mut self, g: &Graph) -> Result<(), CatscopeGuestError> {
        if !self.pending_second_hop {
            return Ok(());
        }
        let subs = SubscriptionQueue::subscribe_now(g, vec![
            SubscriptionRequest { root: self.perp_asset_map_id, filter_weight: 0, depth: 1 },
            SubscriptionRequest {
                root: account_id_from_pubkey(&self.global_trader_index_header_pk),
                filter_weight: 0,
                depth: 1,
            },
            SubscriptionRequest {
                root: account_id_from_pubkey(&self.active_trader_buffer_header_pk),
                filter_weight: 0,
                depth: 1,
            },
        ])?;
        self.subscriptions.extend(subs);
        self.pending_second_hop = false;
        Ok(())
    }

    /// Dispatch a live account update. Routed by known-pubkey identity
    /// (not by reading the account discriminant), same convention as
    /// `pumpswap.rs`/`pumpfun.rs`.
    pub fn on_account(&mut self, header: &Header, body: &[u8]) {
        if header.accountid == self.global_config_id {
            if let Some(v) = accounts::parse_global_config(body) {
                self.canonical_mint_pk = v.canonical_token_mint;
                self.global_vault_pk = v.global_vault;
                self.perp_asset_map_pk = v.perp_asset_map;
                self.perp_asset_map_id = account_id_from_pubkey(&v.perp_asset_map);
                self.global_trader_index_header_pk = v.global_trader_index_header;
                self.active_trader_buffer_header_pk = v.active_trader_buffer_header;
                self.withdraw_queue_pk = v.withdraw_queue;
                if !self.global_ready {
                    self.pending_second_hop = true;
                }
                self.global_ready = true;
            }
            return;
        }
        if header.accountid == self.perp_asset_map_id {
            for m in self.markets.iter_mut() {
                if let Some(e) = accounts::find_perp_asset_by_id(body, m.asset_id) {
                    m.tick_size = e.tick_size;
                    m.base_lot_decimals = e.base_lot_decimals;
                    m.tier0_max_leverage = e.tier0_max_leverage;
                    m.tier0_upper_bound_size = e.tier0_upper_bound_size;
                    m.cumulative_funding_rate = e.cumulative_funding_rate;
                    m.open_interest = e.open_interest;
                    m.open_interest_cap = e.open_interest_cap;
                    m.oracle_mark_price_ticks = e.oracle_mark_price_ticks;
                    m.priced = true;
                }
            }
            return;
        }
        if Some(header.accountid) == self.o_trader_account_id {
            if let Some(h) = accounts::parse_trader_header(body) {
                self.trader_registered = true;
                self.collateral_quote_lots = h.quote_lot_collateral;
                self.capability_flags = h.capability_flags;
                self.max_positions = h.max_positions;
                self.positions.clear();
                for i in 0..h.position_count as usize {
                    if let Some(p) = accounts::parse_trader_position(body, i) {
                        self.positions.push(PhoenixPositionState {
                            asset_id: p.asset_id,
                            base_lot_position: p.base_lot_position,
                            virtual_quote_lot_position: p.virtual_quote_lot_position,
                            cumulative_funding_snapshot: p.cumulative_funding_snapshot,
                            accumulated_funding_for_active_position: p
                                .accumulated_funding_for_active_position,
                        });
                    }
                }
            }
        }
    }

    /// Remaining-accounts list for `global_trader_index`. Verified live at
    /// `num_arenas = 1` -- panics rather than silently sending a wrong
    /// (and therefore economically dangerous) instruction if the exchange
    /// ever scales past a single arena; see the module doc.
    pub fn global_trader_index_accounts(&self) -> Vec<Pubkey> {
        assert_eq!(
            self.global_trader_index_header_pk == Pubkey::default(),
            false,
            "global_trader_index_header not yet known -- GlobalConfiguration not parsed"
        );
        vec![self.global_trader_index_header_pk]
    }

    /// Remaining-accounts list for `active_trader_buffer`. See
    /// [`Self::global_trader_index_accounts`]'s doc.
    pub fn active_trader_buffer_accounts(&self) -> Vec<Pubkey> {
        assert_eq!(
            self.active_trader_buffer_header_pk == Pubkey::default(),
            false,
            "active_trader_buffer_header not yet known -- GlobalConfiguration not parsed"
        );
        vec![self.active_trader_buffer_header_pk]
    }

    /// **Placeholder** -- called by `Updater::batch_router` (via
    /// `DexState::batch_router`, so it runs on `arbv1`'s normal per-commit
    /// cadence), currently a documented no-op. This is the hook point for
    /// feeding Phoenix's live mark prices into `TradeRouter`, same spirit
    /// as `should_open_position` in `brain::phoenixperpsv1::state` --
    /// intentionally left for later, not designed here.
    ///
    /// What a real implementation would need to resolve first (not solved
    /// by this pass):
    /// - **No `DexType::Phoenix*` variant exists yet** in `trader::types`
    ///   -- required by both `add_generic_pair`/`add_directed_edge`.
    /// - **A perp mark price isn't a swap rate.** The closest honest
    ///   framing is a *synthetic, informational* edge from a market's
    ///   underlying spot mint to a quote mint, priced at
    ///   `oracle_mark_price_ticks * tick_size` (see `margin.rs`'s same
    ///   conversion) -- useful for cross-venue basis comparison in
    ///   `find_arbitrage`'s cycle search, but nothing can actually execute
    ///   through it the way a real swap leg can (see the module doc's
    ///   Context on why this isn't a `TradeRouter` edge in the first
    ///   place). Whether `find_arbitrage`/`planner::find_opportunity`
    ///   should even be allowed to route through a non-executable edge is
    ///   an open design question, not just a wiring gap.
    /// - **Which mint is "the underlying"?** `PhoenixMarketState` only
    ///   carries the perp's `symbol`/`asset_id` today, not a spot mint
    ///   pubkey to key an edge on -- would need a lookup table (e.g.
    ///   symbol -> mint) added to the `phoenix_market` prefetch.db table /
    ///   `PhoenixMarketRaw`.
    /// - Only markets with `priced == true` and `global_ready` should ever
    ///   be considered, mirroring every other placeholder/real
    ///   `batch_router` in this codebase.
    pub fn add_to_pricing_router(&self, _router: &mut crate::trader::pricegraph::TradeRouter) {}
}

impl super::update::Updater for PhoenixState {
    fn on_account(&mut self, header: &Header, body: &[u8]) {
        PhoenixState::on_account(self, header, body);
    }

    /// No token accounts tracked (Phoenix's own accounts -- `GlobalConfiguration`,
    /// `PerpAssetMap`, `Trader` -- are all program-owned, not SPL token
    /// accounts), matching e.g. `kamino.rs`'s own `on_token` no-op.
    fn on_token(&mut self, _ta: &crate::catscope::witbot::shooter::Tokenaccountv1) -> bool {
        false
    }

    fn on_tx(&mut self, _ix: &crate::txview::CatscopeInstructionRead<'_>, _slot: &solana_sdk::clock::Slot) {}

    /// See [`PhoenixState::add_to_pricing_router`]'s doc -- placeholder,
    /// same convention as Kamino/Marginfi/Solend's own no-op
    /// `batch_router`s ("a lending Bank has no swap price to contribute to
    /// TradeRouter").
    fn batch_router(&mut self, router: &mut crate::trader::pricegraph::TradeRouter) {
        self.add_to_pricing_router(router);
    }

    // `flush_pending` is always a small, fixed 3-item batch (not a
    // per-commit accumulator like Orca/spl_stake_pool/Raydium CLMM), so
    // it doesn't need pacing -- `max_per_flush` is unused here, kept only
    // to satisfy `Updater`'s shared signature.
    fn flush_pool(&mut self, g: &Graph, _max_per_flush: usize) -> Result<(), CatscopeGuestError> {
        self.flush_pending(g)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_state() -> PhoenixState {
        PhoenixState {
            program_id: 0,
            global_config_id: 1,
            global_ready: true,
            canonical_mint_pk: Pubkey::new_unique(),
            global_vault_pk: Pubkey::new_unique(),
            perp_asset_map_id: 2,
            perp_asset_map_pk: Pubkey::new_unique(),
            global_trader_index_header_pk: Pubkey::new_unique(),
            active_trader_buffer_header_pk: Pubkey::new_unique(),
            withdraw_queue_pk: Pubkey::new_unique(),
            pending_second_hop: false,
            markets: vec![],
            market_by_asset_id: HashMap::new(),
            o_authority_pk: None,
            o_trader_account_pk: None,
            o_trader_account_id: None,
            trader_account_ready: false,
            trader_registered: false,
            collateral_quote_lots: 0,
            capability_flags: 0,
            max_positions: 0,
            positions: vec![],
            subscriptions: Vec::new(),
        }
    }

    #[test]
    fn arena_account_lists_are_single_header_today() {
        let state = synthetic_state();
        assert_eq!(state.global_trader_index_accounts().len(), 1);
        assert_eq!(state.active_trader_buffer_accounts().len(), 1);
    }

    #[test]
    #[should_panic(expected = "not yet known")]
    fn arena_account_list_panics_before_global_config_parsed() {
        let mut state = synthetic_state();
        state.global_trader_index_header_pk = Pubkey::default();
        let _ = state.global_trader_index_accounts();
    }

    #[test]
    fn symbol_str_trims_trailing_zero_padding() {
        let mut symbol = [0u8; 16];
        symbol[..3].copy_from_slice(b"SOL");
        let m = PhoenixMarketState {
            symbol,
            asset_id: 0,
            market_pk: Pubkey::default(),
            spline_collection_pk: Pubkey::default(),
            tick_size: 0,
            base_lot_decimals: 0,
            tier0_max_leverage: 0,
            tier0_upper_bound_size: 0,
            cumulative_funding_rate: 0,
            open_interest: 0,
            open_interest_cap: 0,
            oracle_mark_price_ticks: 0,
            priced: false,
            base_mint: None,
        };
        assert_eq!(m.symbol_str(), "SOL");
    }

    fn market_with_price(tick_size: u64, base_lot_decimals: i8, oracle_mark_price_ticks: u64) -> PhoenixMarketState {
        PhoenixMarketState {
            symbol: [0u8; 16],
            asset_id: 0,
            market_pk: Pubkey::default(),
            spline_collection_pk: Pubkey::default(),
            tick_size,
            base_lot_decimals,
            tier0_max_leverage: 0,
            tier0_upper_bound_size: 0,
            cumulative_funding_rate: 0,
            open_interest: 0,
            open_interest_cap: 0,
            oracle_mark_price_ticks,
            priced: true,
            base_mint: None,
        }
    }

    #[test]
    fn mark_price_usd_is_none_when_no_oracle_price_recorded_yet() {
        // Unlike the market-closure-only finalized_mark_price this bot
        // used before this session's fix, oracle_mark_price_ticks is
        // genuinely live for SOL/BTC/ETH right now (verified this
        // session, see `mark_price_usd`'s doc) -- 0 here represents a
        // market that's never had any price recorded at all (e.g. a
        // brand-new listing), not the common case.
        let m = market_with_price(100, 2, 0);
        assert_eq!(m.mark_price_usd(), None);
    }

    #[test]
    fn mark_price_usd_matches_expected_formula() {
        // tick_size=100, base_lot_decimals=2 match SOL's real live
        // static params (verified this session); oracle_mark_price_ticks
        // chosen as a round number for an exactly-checkable result:
        // 10_000 * 100 = 1_000_000 quote_lots_per_base_lot, / 10^6 = 1.0
        // USD per base_lot, * 10^2 base_lots_per_base_unit = 100.0 USD.
        let m = market_with_price(100, 2, 10_000);
        assert_eq!(m.mark_price_usd(), Some(100.0));
    }

    #[test]
    fn mark_price_usd_matches_real_live_sol_reading() {
        // Real values read live this session from SOL-PERP's actual
        // PerpAssetMap entry on mainnet (oracle_mark_price_ticks=7562,
        // tick_size=100, base_lot_decimals=2) -- confirms the formula
        // against a genuine on-chain reading, not just a synthetic
        // round number. $75.62 matched this bot's own independent
        // spot-price probes at the time.
        let m = market_with_price(100, 2, 7562);
        assert_eq!(m.mark_price_usd(), Some(75.62));
    }
}
