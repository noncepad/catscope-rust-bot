//! Raydium CLMM (concentrated-liquidity market maker) parser and swap builder.
//!
//! # Account layout — Raydium CLMM PoolState (Anchor)
//!
//! ```text
//! offset   size  field
//! ──────   ────  ─────────────────────────────────────────
//!    0       8   Anchor discriminator
//!    8       1   bump (u8)
//!    9      32   amm_config (Pubkey)
//!   41      32   owner (Pubkey)
//!   73      32   token_mint_0 (Pubkey)
//!  105      32   token_mint_1 (Pubkey)
//!  137      32   token_vault_0 (Pubkey)
//!  169      32   token_vault_1 (Pubkey)
//!  201      32   observation_key (Pubkey)
//!  233       1   mint_decimals_0 (u8)
//!  234       1   mint_decimals_1 (u8)
//!  235       2   tick_spacing (u16)
//!  237      16   liquidity (u128)
//!  253      16   sqrt_price_x64 (u128)
//!  269       4   tick_current (i32)
//!  273       2   padding3 (u16)
//!  275       2   padding4 (u16)
//!  277      16   fee_growth_global_0_x64 (u128)
//!  293      16   fee_growth_global_1_x64 (u128)
//!  309       8   protocol_fees_token_0 (u64)
//!  317       8   protocol_fees_token_1 (u64)
//!  325      64   padding5 ([u128; 4])
//!  389       1   status (u8)
//!  390       1   fee_on (u8)
//!  391       2   seed_index ([u8; 2])
//!  393       4   padding ([u8; 4])
//!  397     507   reward_infos ([RewardInfo; 3])
//!  904     128   tick_array_bitmap ([u64; 16])
//! ```
//!
//! `tick_array_bitmap`'s offset (904) was fetched from Raydium's real
//! on-chain source (`raydium-io/raydium-clmm/programs/amm/src/states/
//! pool.rs`) then independently verified against a real live pool account
//! this session: the bit for tick-array window `start=-14400` (a window
//! we separately confirmed via a real fetched `TickArrayState` account
//! has 9 real initialized ticks) decodes as initialized via the vendored,
//! mainnet-verified `check_current_tick_array_is_initialized`, matching.
//! An earlier guess at this offset (273, i.e. immediately after
//! `tick_current` with no gap) was wrong and caught by this exact
//! verification -- there are real padding/fee/reward fields in between
//! that aren't otherwise parsed by this module.
//!
//! # Account layout — Raydium CLMM TickArrayState (Anchor)
//!
//! Verified against a real live tick-array account this session (RPC
//! `getAccountInfo` on a PDA derived from these exact seeds/offsets):
//! `start_tick_index` decoded from the account matched the independently
//! computed value exactly, all 60 ticks showed perfectly tick-spacing-
//! spaced tick numbers, and the total byte layout summed to exactly the
//! account's real on-chain size (10240 bytes). PDA seeds:
//! `[b"tick_array", pool_pubkey, start_tick_index.to_be_bytes()]` (big-
//! endian, unlike Orca Whirlpool's decimal-string seed).
//!
//! ```text
//! offset   size  field
//! ──────   ────  ─────────────────────────────────────────
//!    0       8   Anchor discriminator
//!    8      32   pool_id (Pubkey)
//!   40       4   start_tick_index (i32)
//!   44   60×168  ticks ([TickState; 60])
//! 10124       1  initialized_tick_count (u8, unused here -- see below)
//! 10125       8  recent_epoch (u64)
//! 10133     107  padding
//! ```
//!
//! Each `TickState` (168 bytes): `tick: i32(4)`, `liquidity_net: i128(16)`,
//! `liquidity_gross: u128(16)`, plus 132 bytes of fee/reward/order fields
//! not needed for a swap quote. No explicit "initialized" flag field
//! (unlike Orca's `TickFacade`) -- a tick is initialized iff
//! `liquidity_gross != 0`, the standard convention, confirmed against the
//! live account (9 ticks with nonzero `liquidity_gross`, each showing a
//! plausible `liquidity_net` of matching magnitude). The account's own
//! `initialized_tick_count` field didn't exactly match that count (10 vs
//! 9) in the live sample -- unexplained, but irrelevant here since it's
//! never read; the tick list fed to `compute_swap_full` is always
//! rebuilt by scanning for `liquidity_gross != 0` directly.

use crate::{
    catscope::witbot::shooter::{Header, Tokenaccountv1},
    err::CatscopeGuestError,
    graph::{AccountId, Graph, SubscriptionQueue, SubscriptionRequest},
    log_warn,
    trader::{
        dex::update::Updater,
        pricegraph::{cp_quote, Hop, TradeRouter},
        types::{DexType, PoolPrice, SwapParams, TraderError, RAYDIUM_HOP_MAX_SLIPPAGE},
    },
    util::{account_id_from_pubkey, pubkey_from_account_id},
    wallet::Wallet,
};
use solana_clmm_raydium::{
    compute_swap_full, next_initialized_tick_array_start_index, tick_array_bit_map::max_tick_in_tickarray_bitmap,
    InitializedTick, PoolTickBitmap, SwapPool, MAX_TICK,
};
use solana_sdk::{
    message::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use std::{
    collections::HashMap,
    hash::BuildHasherDefault,
};
use twox_hash::XxHash64;

// ─── Program IDs ─────────────────────────────────────────────────────────────

pub const RAYDIUM_CLMM_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK");

pub const SPL_TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

pub const TOKEN_2022_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

pub const SPL_MEMO_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");

// ─── Anchor discriminator ─────────────────────────────────────────────────────

pub const DISC_POOL_STATE: [u8; 8] = [247, 237, 227, 245, 215, 195, 222, 70];

/// Anchor discriminator for the `swap_v2` instruction: sha256("global:swap_v2")[0..8].
pub const SWAP_V2_DISC: [u8; 8] = [43, 4, 237, 11, 26, 201, 30, 98];

/// Anchor discriminator for `TickArrayState` -- read directly off a real
/// live tick-array account this session (RPC `getAccountInfo`), not
/// computed from `sha256("account:TickArrayState")` -- see this module's
/// doc comment.
pub const DISC_TICK_ARRAY: [u8; 8] = [0xc0, 0x9b, 0x55, 0xcd, 0x31, 0xf9, 0x81, 0x2a];

// ─── Pool state offsets (absolute, including 8-byte discriminator) ────────────

const OFF_AMM_CONFIG: usize = 9;
const OFF_MINT_0: usize = 73;
const OFF_MINT_1: usize = 105;
const OFF_VAULT_0: usize = 137;
const OFF_VAULT_1: usize = 169;
const OFF_OBSERVATION_KEY: usize = 201;
const OFF_DECIMALS_0: usize = 233;
const OFF_DECIMALS_1: usize = 234;
const OFF_TICK_SPACING: usize = 235;
const OFF_LIQUIDITY: usize = 237;
const OFF_SQRT_PRICE: usize = 253;
const OFF_TICK_CURRENT: usize = 269;
/// See this module's doc comment for the verified derivation (fetched
/// from Raydium's real source, then independently confirmed against a
/// real live pool account -- an earlier guess of 273, i.e. immediately
/// after `tick_current` with no intervening fields, was wrong).
const OFF_TICK_ARRAY_BITMAP: usize = 904;

const MIN_POOL_LEN: usize = OFF_TICK_ARRAY_BITMAP + 128; // 1032

// ─── Tick-array state offsets/constants (absolute, incl. discriminator) ───────

/// Ticks stored per tick array. Raydium CLMM uses 60 (Orca Whirlpool uses
/// 88 -- do not conflate the two).
const TICK_ARRAY_SIZE: i32 = 60;
const OFF_TA_POOL_ID: usize = 8;
const OFF_TA_START_INDEX: usize = 40;
const TICKS_OFFSET: usize = 44; // discriminator(8) + pool_id(32) + start_tick_index(4)
/// Per-`TickState` byte size: tick(4) + liquidity_net(16) + liquidity_gross(16)
/// + fee_growth_outside_0/1(16×2) + reward_growths_outside(48) + order/fee
/// fields not needed for a swap quote (60) = 168. Verified against the real
/// account's total size (see module doc).
const TICK_STATE_LEN: usize = 168;
const MIN_TICK_ARRAY_LEN: usize = TICKS_OFFSET + TICK_ARRAY_SIZE as usize * TICK_STATE_LEN; // 10124

/// Parsed state of a Raydium CLMM tick-array account -- only the
/// initialized ticks (`liquidity_gross != 0`) are kept, already in the
/// `{tick, liquidity_net}` shape `compute_swap_full` consumes directly.
/// Mirrors Orca's `ParsedTickArray`, adapted to a `Vec` since the vendor
/// swap-math crate takes a flat slice rather than a fixed-size facade.
#[derive(Clone, Debug)]
pub struct ParsedTickArray {
    pub start_tick_index: i32,
    pub pool_id: AccountId,
    pub ticks: Vec<InitializedTick>,
}

// ─── Parsed pool state ────────────────────────────────────────────────────────
pub struct RaydiumClmm {
    pub program_id: AccountId,
    m_pool: HashMap<AccountId, RaydiumClmmPoolWrapper, BuildHasherDefault<XxHash64>>,
    /// vault account -> the pool it belongs to. Populated as vaults are
    /// discovered from parsed pool accounts (not known up front -- unlike
    /// the pool pubkeys themselves, `RaydiumClmmPoolSetup` doesn't carry
    /// vault addresses).
    m_vault: HashMap<AccountId, AccountId, BuildHasherDefault<XxHash64>>,
    /// tick-array account -> its parsed ticks.
    m_tick_array: HashMap<AccountId, ParsedTickArray, BuildHasherDefault<XxHash64>>,
    /// pool -> { start_tick_index -> tick-array account }.
    m_pool_tick_arrays: HashMap<AccountId, HashMap<i32, AccountId>, BuildHasherDefault<XxHash64>>,
    /// Tick-array subscriptions queued by a window shift in `on_account`,
    /// paced through in `flush_pool` -- mirrors Orca's
    /// `token_sub_queue`/`flush_pool` pattern exactly (tick arrays are
    /// PDA-derived, not graph-discoverable, so they need explicit
    /// subscription like Orca's). Keeps resulting `Subscription`s alive
    /// internally, same as every other `SubscriptionQueue` user.
    tick_array_sub_queue: SubscriptionQueue,
}
// SubscriptionRequest (inside tick_array_sub_queue) doesn't implement
// Debug -- same reason `OrcaState` (which has the equivalent field)
// also skips #[derive(Debug)] in favor of this minimal manual impl.
impl std::fmt::Debug for RaydiumClmm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RaydiumClmm").finish()
    }
}
impl RaydiumClmm {
    /// Builds this dex's live state and returns its pending subscription
    /// requests alongside it -- doesn't subscribe itself. See
    /// `RaydiumAmm::new`'s doc comment for why (paced through a shared
    /// [`crate::graph::SubscriptionQueue`] owned by `DexState` instead).
    pub fn new(setups: &[RaydiumClmmPoolSetup]) -> (Self, Vec<SubscriptionRequest>) {
        let program_id = account_id_from_pubkey(&RAYDIUM_CLMM_PROGRAM_ID);
        let mut m_pool =
            HashMap::with_capacity_and_hasher(setups.len(), BuildHasherDefault::default());
        let mut l_req = Vec::with_capacity(setups.len());

        for setup in setups {
            l_req.push(SubscriptionRequest {
                root: setup.pubkey,
                filter_weight: u32::MAX,
                depth: 1,
            });
            m_pool.insert(
                setup.pubkey,
                RaydiumClmmPoolWrapper {
                    pool: RaydiumClmmPool {
                        token_mint_0: setup.mint_0,
                        token_mint_1: setup.mint_1,
                        ..RaydiumClmmPool::default()
                    },
                    vault_0_balance: 0,
                    vault_1_balance: 0,
                    fee_rate_pips: setup.fee_rate_pips,
                    subscribed_tick_start: None,
                },
            );
        }
        let state = Self {
            program_id,
            m_pool,
            m_vault: HashMap::with_capacity_and_hasher(
                setups.len() * 2,
                BuildHasherDefault::default(),
            ),
            m_tick_array: HashMap::default(),
            m_pool_tick_arrays: HashMap::default(),
            tick_array_sub_queue: SubscriptionQueue::default(),
        };
        (state, l_req)
    }
    pub fn program_id(&self) -> &AccountId {
        &self.program_id
    }
    pub fn pool_count(&self) -> usize {
        self.m_pool.len()
    }

    /// TEMPORARY DIAGNOSTIC (2026-09-07): how many tick-array subscribe
    /// requests `tick_array_sub_queue` currently has queued but not yet
    /// flushed -- same motivation as `OrcaState::token_sub_queue_pending_
    /// count`, checking whether a chronically `PoolNotReady` Raydium CLMM
    /// pool (real, live-confirmed against pool 687973351's own on-chain
    /// bitmap, which proves its tick arrays genuinely exist -- see
    /// `plan_hop`'s doc comment) is stuck behind a real backlog in the
    /// shared 128/cycle `flush_pool` budget.
    pub fn tick_array_sub_queue_pending_count(&self) -> usize {
        self.tick_array_sub_queue.pending_count()
    }

    pub fn populate_router(&self, router: &mut TradeRouter) {
        for (&pool_id, wrapper) in &self.m_pool {
            router.add_raydium_clmm_pool(pool_id, &wrapper.pool, wrapper.fee_rate_pips, DexType::RaydiumClmm);
        }
    }

    /// Build the swap instruction for one `Hop` routed through this dex --
    /// same uniform adapter shape as every other dex module's `plan_hop`.
    pub fn plan_hop(
        &self,
        hop: &Hop,
        owner: AccountId,
        source_ata: AccountId,
        dest_ata: AccountId,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let wrapper = self.m_pool.get(&hop.pool_id).ok_or(TraderError::UnknownPool(hop.pool_id))?;
        let pool = &wrapper.pool;

        // Real, live-confirmed failure mode (2026-08-27, same session/root
        // cause as `OrcaState::swap`'s identical fix -- see that
        // function's doc comment): `build_swap_ix` derives its tick-array
        // accounts as PDAs from `tick_array_windows`'s bitmap-computed
        // start indices, unconditionally, regardless of whether this bot
        // has ever actually subscribed to/decoded those specific
        // tick-array accounts. Unlike Orca's naive fixed-offset guess,
        // these starts *are* real (bitmap-derived, so the accounts really
        // are initialized on-chain) -- but sending a swap referencing
        // tick arrays this bot has zero real local data for is still
        // flying blind: real, live-confirmed outcome was an on-chain
        // `NotEnoughTickArrayAccount` revert (a real transaction fee paid
        // to find that out). Refuse before ever reaching `build_swap_ix`
        // if any of the windows we're about to reference have no real
        // subscribed tick-array data yet, same "refuse rather than
        // guess" policy as the Orca fix, same error type.
        let zero_for_one = if hop.input_mint == pool.token_mint_0 && hop.output_mint == pool.token_mint_1 {
            true
        } else if hop.input_mint == pool.token_mint_1 && hop.output_mint == pool.token_mint_0 {
            false
        } else {
            return Err(TraderError::WrongMints);
        };
        let Some(ta_starts) = tick_array_windows(pool, zero_for_one) else {
            return Err(TraderError::PoolNotReady);
        };
        let m_starts = self.m_pool_tick_arrays.get(&hop.pool_id);
        for start in ta_starts {
            let has_data = m_starts
                .and_then(|m| m.get(&start))
                .is_some_and(|ta_id| self.m_tick_array.contains_key(ta_id));
            if !has_data {
                log_warn!(
                    "raydium clmm: pool={} has no real tick-array data for window start={} -- refusing rather than guessing a PDA",
                    hop.pool_id,
                    start,
                );
                return Err(TraderError::PoolNotReady);
            }
        }

        let mut params = SwapParams {
            pool: hop.pool_id,
            input_mint: hop.input_mint,
            output_mint: hop.output_mint,
            amount_in: hop.amount_in,
            min_amount_out: hop.amount_out,
            user_source_token_account: source_ata,
            user_destination_token_account: dest_ata,
            user_wallet: owner,
        };
        // See SwapParams::apply_slippage_tolerance's doc comment -- without
        // this, real on-chain price movement between quote and landing
        // reliably trips the pool's own minimum-output check.
        params.apply_slippage_tolerance(RAYDIUM_HOP_MAX_SLIPPAGE);
        build_swap_ix(hop.pool_id, &wrapper.pool, &params, wallet)
    }

    /// Exact tick-aware CLMM quote for a single amount, via
    /// `solana_clmm_raydium::compute_swap_full` -- NOT the naive
    /// constant-product-within-current-tick approximation `cp_quote`/
    /// `TradeRouter::requote_edge` use for cheap graph search. Mirrors
    /// `OrcaState::exact_quote` exactly (same missing-tick-array
    /// graceful-degradation behavior: a window with no subscribed/decoded
    /// tick array yet is simply omitted from `initialized_ticks`, not
    /// treated as an error).
    ///
    /// Real usage: `planner::reverify_hops` calls this to correct a
    /// router-found hop through a Raydium CLMM pool before trusting it --
    /// found this session that without this, a hop could pass the
    /// constant-product re-quote fine while being ~94% off the real
    /// tick-aware price.
    ///
    /// Returns `None` if the pool isn't known, has zero active liquidity,
    /// `input_mint` doesn't match either side of the pool, the swap would
    /// need the `TickArrayBitmapExtension` account (see
    /// `tick_array_windows`'s doc comment), or the underlying quote call
    /// errors.
    pub fn exact_quote(&self, pool_id: AccountId, input_mint: AccountId, amount_in: u64) -> Option<u64> {
        let wrapper = self.m_pool.get(&pool_id)?;
        let pool = &wrapper.pool;
        if pool.liquidity == 0 {
            return None;
        }
        let zero_for_one = if input_mint == pool.token_mint_0 {
            true
        } else if input_mint == pool.token_mint_1 {
            false
        } else {
            return None;
        };

        let starts = tick_array_windows(pool, zero_for_one)?;

        let mut ticks: Vec<InitializedTick> = Vec::new();
        let m_starts = self.m_pool_tick_arrays.get(&pool_id);
        for &start in &starts {
            let Some(ta_id) = m_starts.and_then(|m| m.get(&start)) else { continue };
            let Some(pta) = self.m_tick_array.get(ta_id) else { continue };
            ticks.extend_from_slice(&pta.ticks);
        }
        ticks.sort_by_key(|t| t.tick);

        let swap_pool = SwapPool {
            sqrt_price_x64: pool.sqrt_price_x64,
            liquidity: pool.liquidity,
            tick_current: pool.tick_current,
            tick_spacing: pool.tick_spacing,
            fee_rate_pips: wrapper.fee_rate_pips,
        };
        let result = compute_swap_full(&swap_pool, &ticks, amount_in, 0, true, zero_for_one).ok()?;
        Some(result.amount_out)
    }

    /// Whether every tick-array window [`Self::exact_quote`] needs for this
    /// `pool_id`/`input_mint` direction is actually cached in
    /// `m_tick_array` right now -- same purpose and same real, live-
    /// confirmed incident (2026-09-04) as `OrcaWhirlpool::exact_quote_ready`'s
    /// doc comment; `exact_quote` here has the identical "a window with no
    /// subscribed/decoded tick array yet is simply omitted from
    /// `initialized_ticks`" degradation, which looks identical to genuine
    /// thin/zero liquidity to `compute_swap_full`. Callers should check
    /// this before trusting an `exact_quote(None)`/`Some(0)` as a real
    /// rejection worth a cooldown -- see `planner::reverify_hops`'s doc
    /// comment.
    pub fn exact_quote_ready(&self, pool_id: AccountId, input_mint: AccountId) -> bool {
        let Some(wrapper) = self.m_pool.get(&pool_id) else {
            return false;
        };
        let pool = &wrapper.pool;
        if pool.liquidity == 0 {
            // Genuinely no active liquidity -- not a readiness question.
            return true;
        }
        let zero_for_one = if input_mint == pool.token_mint_0 {
            true
        } else if input_mint == pool.token_mint_1 {
            false
        } else {
            return false;
        };
        let Some(starts) = tick_array_windows(pool, zero_for_one) else {
            // Needs the TickArrayBitmapExtension account -- same "not
            // ready yet" situation as a missing tick array.
            return false;
        };
        let m_starts = self.m_pool_tick_arrays.get(&pool_id);
        starts.iter().all(|start| {
            m_starts
                .and_then(|m| m.get(start))
                .is_some_and(|ta_id| self.m_tick_array.contains_key(ta_id))
        })
    }
}

/// Compute the start tick index for the tick array covering `tick`.
/// Mirrors `OrcaWhirlpool::tick_array_start` exactly (same floor-toward-
/// negative-infinity division), just parameterized by `tick_spacing`
/// directly instead of reading `&self`.
fn tick_array_start(tick: i32, tick_spacing: u16) -> i32 {
    let size = TICK_ARRAY_SIZE * tick_spacing as i32;
    if tick >= 0 {
        (tick / size) * size
    } else {
        ((tick - size + 1) / size) * size
    }
}

/// Real tick-array windows for a swap in `zero_for_one` direction,
/// walked via the pool's on-chain bitmap using
/// `next_initialized_tick_array_start_index` -- the same lookup the real
/// on-chain `swap_v2` handler itself uses to decide which tick arrays to
/// consume next -- instead of blindly guessing 3 adjacent windows exist.
/// The first entry is always the current window (loaded unconditionally
/// by the on-chain program regardless of its own initialized status);
/// subsequent entries are the next real initialized windows found by the
/// walk, up to 3 total (matching `exact_quote`'s/`build_swap_ix`'s
/// existing 3-tick-array budget).
///
/// Returns `None` -- a deliberate decline, not a best-effort guess -- if
/// the walk runs off the edge of the pool's default (non-extension)
/// bitmap coverage on a pool whose `tick_spacing` doesn't give that
/// default bitmap full coverage of the entire valid tick range. In that
/// case the `TickArrayBitmapExtension` account might hold further real
/// initialized ticks this function has no way to see, and the on-chain
/// program would additionally require that account's pubkey appended to
/// the instruction's remaining accounts -- neither of which the vendored,
/// mainnet-verified `solana_clmm_raydium` swap-math crate has any support
/// for (it's pure math, no account decoding, and doesn't implement the
/// extension's separate bitmap-walk logic at all). Rather than hand-roll
/// new, unverified bitmap arithmetic to cover that gap, this declines
/// cleanly. Most real high-volume pools (`tick_spacing` 60+) have full
/// default-bitmap coverage and never hit this path -- verified directly:
/// `max_tick_in_tickarray_bitmap(60) = 1,843,200`, far beyond `MAX_TICK`
/// (443,636), so a `tick_spacing=60` pool's default bitmap always covers
/// the entire valid price range and this function can never decline for
/// it. Only very fine-grained pools (small `tick_spacing`, e.g. 1) can
/// have partial coverage and hit this path.
fn tick_array_windows(pool: &RaydiumClmmPool, zero_for_one: bool) -> Option<[i32; 3]> {
    let bitmap = PoolTickBitmap::new(pool.tick_array_bitmap);
    let start_0 = tick_array_start(pool.tick_current, pool.tick_spacing);
    let full_coverage = max_tick_in_tickarray_bitmap(pool.tick_spacing) >= MAX_TICK;

    let mut starts = [start_0; 3];
    let mut last = start_0;
    for slot in starts.iter_mut().skip(1) {
        let (found, next) =
            next_initialized_tick_array_start_index(&bitmap, last, pool.tick_spacing, zero_for_one);
        if !found {
            if !full_coverage {
                return None;
            }
            // Genuine end of the pool's valid price range in this
            // direction (not a coverage gap) -- leave the remaining
            // slot(s) at the last real window found; a duplicate start
            // index is harmless for `compute_swap_full`'s own off-chain
            // estimate (it finds a boundary tick by value via `.find()`,
            // doesn't sum duplicates) but is NOT harmless once turned
            // into real account metas -- see `build_swap_ix`'s own dedup
            // step, added after a real on-chain panic this duplicate
            // caused (`already mutably borrowed: BorrowError`, the same
            // tick-array PDA appearing twice in one instruction's account
            // list).
            break;
        }
        *slot = next;
        last = next;
    }
    Some(starts)
}

/// Derive a Raydium CLMM tick-array PDA. Seeds verified against a real
/// live account this session -- see this module's doc comment. Unlike
/// Orca's decimal-string seed, Raydium uses the raw big-endian bytes of
/// `start_index`.
fn tick_array_pda(pool_pk: &Pubkey, start_index: i32) -> Pubkey {
    Pubkey::find_program_address(
        &[b"tick_array", pool_pk.as_ref(), &start_index.to_be_bytes()],
        &RAYDIUM_CLMM_PROGRAM_ID,
    )
    .0
}

/// Drops duplicate entries from `starts`, preserving order of first
/// occurrence -- `tick_array_windows` can legitimately return a repeated
/// `start` (genuine end of the pool's tick range in this direction, see
/// its own doc comment), which is harmless for `compute_swap_full`'s
/// off-chain quote estimate (it finds a boundary tick by value via
/// `.find()`, doesn't sum duplicates) but is fatal once turned into real
/// account metas: the same tick-array PDA appearing twice in one
/// instruction's account list makes the real on-chain `swap_v2` handler
/// panic (`already mutably borrowed: BorrowError`) trying to borrow the
/// same account twice -- live-confirmed via a real failed transaction (a
/// close attempt on a thin pool hit exactly this direction-dependent
/// range edge). `starts` only has 3 entries, so a linear "seen" scan is
/// trivial -- no need for a `HashSet`.
fn dedup_tick_array_starts(starts: [i32; 3]) -> Vec<i32> {
    let mut out: Vec<i32> = Vec::with_capacity(starts.len());
    for start in starts {
        if !out.contains(&start) {
            out.push(start);
        }
    }
    out
}

impl Updater for RaydiumClmm {
    fn on_account(&mut self, header: &Header, body: &[u8]) {
        if header.owner != self.program_id {
            return;
        }
        if body.len() >= 8 && body[..8] == DISC_TICK_ARRAY {
            self.on_tick_array_account(header.accountid, body);
            return;
        }
        let Some(wrapper) = self.m_pool.get_mut(&header.accountid) else {
            return;
        };
        let Some(mut new_state) = parse(body) else {
            return;
        };

        // Preserve live reserves from token events -- parse() always
        // zeroes them since they don't live in the account bytes.
        new_state.reserve_0 = wrapper.pool.reserve_0;
        new_state.reserve_1 = wrapper.pool.reserve_1;

        let (old_v0, old_v1) = (wrapper.pool.token_vault_0, wrapper.pool.token_vault_1);
        let (vault_0, vault_1) = (new_state.token_vault_0, new_state.token_vault_1);
        let pool_id = header.accountid;
        wrapper.pool = new_state;

        if old_v0 == 0 && vault_0 != 0 {
            self.m_vault.insert(vault_0, pool_id);
        }
        if old_v1 == 0 && vault_1 != 0 {
            self.m_vault.insert(vault_1, pool_id);
        }

        // Tick arrays are off-chain PDA-derived (not a pubkey field inside
        // the pool account), so the host's account graph has no way to
        // discover them on its own -- subscribe explicitly, same pattern
        // as the vault subscriptions above. Re-derive and re-subscribe
        // whenever price drifts into a new tick-array window, not just
        // once at pool discovery, mirroring
        // `OrcaState::on_account`'s identical tick-array resubscription
        // logic (see that code's own comment for why a stale cached
        // window silently degrades quote accuracy). Subscribes to the
        // real bitmap-directed windows both `exact_quote` and
        // `build_swap_ix` will actually reference (see
        // `tick_array_windows`'s doc comment) -- both directions, since
        // either could be queried -- rather than blindly guessing 5
        // adjacent windows exist.
        let wrapper = self.m_pool.get_mut(&pool_id).expect("just inserted above");
        let start_0 = tick_array_start(wrapper.pool.tick_current, wrapper.pool.tick_spacing);
        if wrapper.subscribed_tick_start != Some(start_0) {
            if let Some(pool_pk) = pubkey_from_account_id(&pool_id) {
                let mut starts: Vec<i32> = vec![start_0];
                if let Some(fwd) = tick_array_windows(&wrapper.pool, false) {
                    starts.extend_from_slice(&fwd);
                }
                if let Some(back) = tick_array_windows(&wrapper.pool, true) {
                    starts.extend_from_slice(&back);
                }
                starts.sort_unstable();
                starts.dedup();
                for start in starts {
                    let ta_pk = tick_array_pda(&pool_pk, start);
                    self.tick_array_sub_queue.push(SubscriptionRequest {
                        root: account_id_from_pubkey(&ta_pk),
                        filter_weight: u32::MAX,
                        depth: 1,
                    });
                }
                self.m_pool.get_mut(&pool_id).unwrap().subscribed_tick_start = Some(start_0);
            }
        }
    }

    fn on_token(&mut self, ta: &Tokenaccountv1) -> bool {
        let Some(&pool_id) = self.m_vault.get(&ta.id) else {
            return false;
        };
        let Some(wrapper) = self.m_pool.get_mut(&pool_id) else {
            return false;
        };
        if ta.id == wrapper.pool.token_vault_0 {
            wrapper.pool.reserve_0 = ta.amount;
        } else if ta.id == wrapper.pool.token_vault_1 {
            wrapper.pool.reserve_1 = ta.amount;
        }
        true
    }

    fn batch_router(&mut self, router: &mut TradeRouter) {
        self.populate_router(router);
    }

    fn refresh_account_router(&mut self, account_id: AccountId, router: &mut TradeRouter) {
        if let Some(wrapper) = self.m_pool.get(&account_id) {
            router.add_raydium_clmm_pool(account_id, &wrapper.pool, wrapper.fee_rate_pips, DexType::RaydiumClmm);
        }
    }

    fn refresh_token_router(&mut self, ta_id: AccountId, router: &mut TradeRouter) {
        if let Some(&pool_id) = self.m_vault.get(&ta_id) {
            if let Some(wrapper) = self.m_pool.get(&pool_id) {
                router.add_raydium_clmm_pool(pool_id, &wrapper.pool, wrapper.fee_rate_pips, DexType::RaydiumClmm);
            }
        }
    }

    fn on_tx(
        &mut self,
        ix: &crate::txview::CatscopeInstructionRead<'_>,
        _slot: &solana_sdk::clock::Slot,
    ) {
        // Decode a landed swap_v2 instruction and provisionally ("fudge")
        // adjust the pool's reserves ahead of the real account update that
        // will arrive ~50ms later. Safe to mutate in place: reserve_0/
        // reserve_1 are the exact same fields on_token writes, so the
        // next real vault-balance update overwrites this estimate with
        // ground truth -- no separate expiry needed.
        if *ix.program() != self.program_id {
            return;
        }
        let data = ix.data();
        let accounts = ix.account();
        if data.len() < 41 || data[..8] != SWAP_V2_DISC || accounts.len() <= 6 {
            return;
        }
        let is_base_input = data[40] != 0;
        // Exact-out swaps use the amount field as the desired output, not
        // input -- not handled here.
        if !is_base_input {
            return;
        }
        let amount_in = u64::from_le_bytes(data[8..16].try_into().unwrap());
        let pool_id = accounts[2];
        let input_vault = accounts[5];
        let Some(wrapper) = self.m_pool.get_mut(&pool_id) else {
            return;
        };
        let fee_bps = (wrapper.fee_rate_pips / 100) as u16;
        let pool = &mut wrapper.pool;
        let zero_for_one = if input_vault == pool.token_vault_0 {
            true
        } else if input_vault == pool.token_vault_1 {
            false
        } else {
            return;
        };
        let (reserve_in, reserve_out) = if zero_for_one {
            (pool.reserve_0, pool.reserve_1)
        } else {
            (pool.reserve_1, pool.reserve_0)
        };
        if reserve_in == 0 || reserve_out == 0 {
            return;
        }
        let amount_out = cp_quote(amount_in, reserve_in, reserve_out, fee_bps);
        let new_in = reserve_in.saturating_sub(amount_in);
        let new_out = reserve_out.saturating_add(amount_out);
        if zero_for_one {
            pool.reserve_0 = new_in;
            pool.reserve_1 = new_out;
        } else {
            pool.reserve_1 = new_in;
            pool.reserve_0 = new_out;
        }
    }

    // Paced through `tick_array_sub_queue` -- see `orca.rs::flush_pool`'s
    // doc comment for why an unbounded per-commit batch here is exactly
    // the bug that produced this session's real `stdio timeout` hangs.
    fn flush_pool(&mut self, g: &Graph, max_per_flush: usize) -> Result<(), CatscopeGuestError> {
        self.tick_array_sub_queue.flush(g, max_per_flush)?;
        Ok(())
    }
}

impl RaydiumClmm {
    /// Decode a `TickArrayState` account (identified by discriminator in
    /// `on_account`, dispatched here) -- keeps only initialized ticks
    /// (`liquidity_gross != 0`), already in the `{tick, liquidity_net}`
    /// shape `compute_swap_full` consumes. See this module's doc comment
    /// for the verified byte layout.
    fn on_tick_array_account(&mut self, ta_id: AccountId, body: &[u8]) {
        if body.len() < MIN_TICK_ARRAY_LEN {
            return;
        }
        let pool_pk = Pubkey::new_from_array(body[OFF_TA_POOL_ID..OFF_TA_POOL_ID + 32].try_into().unwrap());
        let pool_id = account_id_from_pubkey(&pool_pk);
        let start_tick_index =
            i32::from_le_bytes(body[OFF_TA_START_INDEX..OFF_TA_START_INDEX + 4].try_into().unwrap());

        let mut ticks = Vec::new();
        for i in 0..TICK_ARRAY_SIZE as usize {
            let o = TICKS_OFFSET + i * TICK_STATE_LEN;
            if o + TICK_STATE_LEN > body.len() {
                break;
            }
            let liquidity_gross = u128::from_le_bytes(body[o + 20..o + 36].try_into().unwrap());
            if liquidity_gross == 0 {
                continue;
            }
            let tick = i32::from_le_bytes(body[o..o + 4].try_into().unwrap());
            let liquidity_net = i128::from_le_bytes(body[o + 4..o + 20].try_into().unwrap());
            ticks.push(InitializedTick { tick, liquidity_net });
        }

        self.m_tick_array.insert(ta_id, ParsedTickArray { start_tick_index, pool_id, ticks });
        self.m_pool_tick_arrays.entry(pool_id).or_default().insert(start_tick_index, ta_id);
    }
}

#[derive(Debug, Default)]
struct RaydiumClmmPoolWrapper {
    pub pool: RaydiumClmmPool,
    pub vault_0_balance: u64,
    pub vault_1_balance: u64,
    pub fee_rate_pips: u32,
    /// The tick-array window (`tick_array_start` value) this pool's tick
    /// arrays were last subscribed for -- `None` until the first pool
    /// account update. Mirrors Orca's `PoolInfo::subscribed_tick_start`.
    pub subscribed_tick_start: Option<i32>,
}
#[derive(Debug, Clone, Default)]
pub struct RaydiumClmmPool {
    pub token_mint_0: AccountId,
    pub token_mint_1: AccountId,
    pub token_vault_0: AccountId,
    pub token_vault_1: AccountId,
    pub amm_config: AccountId,
    pub observation_key: AccountId,
    pub mint_decimals_0: u8,
    pub mint_decimals_1: u8,
    pub tick_spacing: u16,
    pub liquidity: u128,
    pub sqrt_price_x64: u128,
    pub tick_current: i32,
    /// Raw on-chain tick-array bitmap -- which tick-array windows (within
    /// the default ±512-window range) actually have an initialized
    /// account, per `PoolTickBitmap`'s doc comment. Used by
    /// `tick_array_windows` to find real windows via the vendored,
    /// mainnet-verified bitmap-walk math instead of blindly guessing
    /// adjacent windows exist.
    pub tick_array_bitmap: [u64; 16],
    /// Token vault 0 balance (raw).
    pub reserve_0: u64,
    /// Token vault 1 balance (raw).
    pub reserve_1: u64,
}

impl RaydiumClmmPool {
    /// Spot price: raw token_1 units per raw token_0 unit.
    pub fn spot_price(&self) -> f64 {
        let sqrt = self.sqrt_price_x64 as f64 / (1u128 << 64) as f64;
        sqrt * sqrt
    }

    pub fn pool_price(&self, fee_rate_pips: u32) -> Option<PoolPrice> {
        if self.liquidity == 0 {
            return None;
        }
        let fee_bps = (fee_rate_pips / 100).min(10_000) as u16;
        Some(PoolPrice {
            token_a: self.token_mint_0,
            token_b: self.token_mint_1,
            price: self.spot_price(),
            reserve_a: self.reserve_0,
            reserve_b: self.reserve_1,
            fee_bps,
        })
    }
}

// ─── Setup loaded from raydium_clmm.json ─────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RaydiumClmmPoolSetup {
    pub pubkey: AccountId,
    pub mint_0: AccountId,
    pub mint_1: AccountId,
    pub fee_rate_pips: u32,
}

// ─── Parser ───────────────────────────────────────────────────────────────────

pub fn parse(body: &[u8]) -> Option<RaydiumClmmPool> {
    if body.len() < MIN_POOL_LEN {
        return None;
    }
    if body[..8] != DISC_POOL_STATE {
        return None;
    }
    let read_u8 = |off: usize| body[off];
    let read_u16 = |off: usize| u16::from_le_bytes(body[off..off + 2].try_into().unwrap());
    let read_i32 = |off: usize| i32::from_le_bytes(body[off..off + 4].try_into().unwrap());
    let read_u128 = |off: usize| u128::from_le_bytes(body[off..off + 16].try_into().unwrap());
    let read_pk = |off: usize| Pubkey::new_from_array(body[off..off + 32].try_into().unwrap());
    let pk_id = |pk: Pubkey| account_id_from_pubkey(&pk);
    let mut tick_array_bitmap = [0u64; 16];
    for (i, limb) in tick_array_bitmap.iter_mut().enumerate() {
        let o = OFF_TICK_ARRAY_BITMAP + i * 8;
        *limb = u64::from_le_bytes(body[o..o + 8].try_into().unwrap());
    }

    Some(RaydiumClmmPool {
        amm_config: pk_id(read_pk(OFF_AMM_CONFIG)),
        token_mint_0: pk_id(read_pk(OFF_MINT_0)),
        token_mint_1: pk_id(read_pk(OFF_MINT_1)),
        token_vault_0: pk_id(read_pk(OFF_VAULT_0)),
        token_vault_1: pk_id(read_pk(OFF_VAULT_1)),
        observation_key: pk_id(read_pk(OFF_OBSERVATION_KEY)),
        mint_decimals_0: read_u8(OFF_DECIMALS_0),
        mint_decimals_1: read_u8(OFF_DECIMALS_1),
        tick_spacing: read_u16(OFF_TICK_SPACING),
        liquidity: read_u128(OFF_LIQUIDITY),
        sqrt_price_x64: read_u128(OFF_SQRT_PRICE),
        tick_current: read_i32(OFF_TICK_CURRENT),
        tick_array_bitmap,
        reserve_0: 0,
        reserve_1: 0,
    })
}

// ─── Swap instruction ─────────────────────────────────────────────────────────

pub const RAYDIUM_CLMM_SWAP_CU: u32 = 300_000;

pub fn build_swap_ix(
    pool_id: AccountId,
    pool: &RaydiumClmmPool,
    params: &SwapParams,
    wallet: &mut Wallet,
) -> Result<(), TraderError> {
    let zero_for_one = if params.input_mint == pool.token_mint_0
        && params.output_mint == pool.token_mint_1
    {
        true
    } else if params.input_mint == pool.token_mint_1 && params.output_mint == pool.token_mint_0 {
        false
    } else {
        return Err(TraderError::WrongMints);
    };

    let resolve = |id: AccountId| -> Result<Pubkey, TraderError> {
        pubkey_from_account_id(&id).ok_or(TraderError::PubkeyResolutionFailed(id))
    };

    let pool_pk = resolve(pool_id)?;
    let amm_config_pk = resolve(pool.amm_config)?;
    let observation_pk = resolve(pool.observation_key)?;
    let vault_0_pk = resolve(pool.token_vault_0)?;
    let vault_1_pk = resolve(pool.token_vault_1)?;
    let mint_0_pk = resolve(pool.token_mint_0)?;
    let mint_1_pk = resolve(pool.token_mint_1)?;
    let user_source_pk = resolve(params.user_source_token_account)?;
    let user_dest_pk = resolve(params.user_destination_token_account)?;
    let user_wallet_pk = resolve(params.user_wallet)?;

    let (input_vault_pk, output_vault_pk, input_mint_pk, output_mint_pk) = if zero_for_one {
        (vault_0_pk, vault_1_pk, mint_0_pk, mint_1_pk)
    } else {
        (vault_1_pk, vault_0_pk, mint_1_pk, mint_0_pk)
    };

    // swap_v2: disc(8) + amount(8) + other_amount_threshold(8) + sqrt_price_limit(16) + is_base_input(1)
    let mut data = Vec::with_capacity(41);
    data.extend_from_slice(&SWAP_V2_DISC);
    data.extend_from_slice(&params.amount_in.to_le_bytes());
    data.extend_from_slice(&params.min_amount_out.to_le_bytes());
    data.extend_from_slice(&0u128.to_le_bytes()); // sqrt_price_limit = 0 (no limit)
    data.push(1u8); // is_base_input = true

    let mut accounts = vec![
        AccountMeta::new_readonly(user_wallet_pk, true), // payer
        AccountMeta::new_readonly(amm_config_pk, false), // amm_config
        AccountMeta::new(pool_pk, false),                // pool_state
        AccountMeta::new(user_source_pk, false),         // input_token_account
        AccountMeta::new(user_dest_pk, false),           // output_token_account
        AccountMeta::new(input_vault_pk, false),         // input_vault
        AccountMeta::new(output_vault_pk, false),        // output_vault
        AccountMeta::new(observation_pk, false),         // observation_state
        AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false), // token_program
        AccountMeta::new_readonly(TOKEN_2022_PROGRAM_ID, false), // token_program_2022
        AccountMeta::new_readonly(SPL_MEMO_PROGRAM_ID, false), // memo_program
        AccountMeta::new_readonly(input_mint_pk, false), // input_vault_mint
        AccountMeta::new_readonly(output_mint_pk, false), // output_vault_mint
    ];

    // Tick-array accounts as "remaining accounts" -- required by the
    // real on-chain `swap_v2` handler (verified against
    // raydium-io/raydium-clmm's instruction source this session: it
    // consumes them sequentially from remaining_accounts as the swap
    // crosses tick boundaries). Without these, a swap that needs to
    // cross even one tick array boundary fails on-chain -- this was a
    // real, separate gap from the exact-quote fix (`exact_quote` only
    // needed tick data for an off-chain estimate; this is what the
    // transaction itself requires to execute at all). Same bitmap-
    // directed window selection `exact_quote` uses (see
    // `tick_array_windows`'s doc comment), so the quote and the actual
    // instruction reference the same tick arrays. No
    // `TickArrayBitmapExtension` account handling -- a pool needing it
    // is declined below rather than built with a guessed/incomplete
    // account list, same scope decision as `exact_quote`.
    let Some(ta_starts) = tick_array_windows(pool, zero_for_one) else {
        return Err(TraderError::MissingConfig(
            "raydium clmm: pool needs TickArrayBitmapExtension, not supported",
        ));
    };
    for start in dedup_tick_array_starts(ta_starts) {
        accounts.push(AccountMeta::new(tick_array_pda(&pool_pk, start), false));
    }

    wallet.require_signer(params.user_wallet);
    wallet.append_ix(
        Instruction {
            program_id: RAYDIUM_CLMM_PROGRAM_ID,
            accounts,
            data,
        },
        RAYDIUM_CLMM_SWAP_CU,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_tick_array_starts_drops_a_trailing_duplicate() {
        // Real shape `tick_array_windows` produces at a genuine range
        // edge: the first two windows are real and distinct, the third
        // repeats the second because there's no further initialized
        // tick array in this direction.
        assert_eq!(dedup_tick_array_starts([100, 200, 200]), vec![100, 200]);
    }

    #[test]
    fn dedup_tick_array_starts_drops_a_duplicate_of_the_first_window() {
        // Live-confirmed real shape (the exact failed transaction this
        // fix was built from): the range edge hit on the *second* step,
        // so the third window repeats the *first*, not the second.
        assert_eq!(dedup_tick_array_starts([100, 200, 100]), vec![100, 200]);
    }

    #[test]
    fn dedup_tick_array_starts_keeps_three_distinct_windows() {
        assert_eq!(dedup_tick_array_starts([100, 200, 300]), vec![100, 200, 300]);
    }

    #[test]
    fn dedup_tick_array_starts_collapses_to_one_when_all_equal() {
        // The narrowest real case: the pool's current tick is already at
        // the very edge of its populated range, so every window is the
        // same starting one.
        assert_eq!(dedup_tick_array_starts([100, 100, 100]), vec![100]);
    }
}

