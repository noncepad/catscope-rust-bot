//! TSLAx (Tesla xStock, a real Backed Finance Token-2022 mint) round-trip
//! swap test against its own live Raydium CLMM pool.
//!
//! Real, live-verified 2026-09-16: pool `HHQUnUbmWLrYzkscDY1C3deEFbGtiGBGoHjpANogmvum`
//! -- fetched directly (`getAccountInfo`) and decoded against `raydium::
//! clmm`'s own verified `PoolState` offsets: `token_mint_0` = TSLAx
//! (`XsDoVfqeBukxuZHWhdvWHBhgEHjGNst4MLodqsJHzoB`, decimals 8, real
//! Token-2022 mint with `freezeAuthority`/`permanentDelegate`/
//! `pausableConfig` extensions all live -- see this session's own
//! research), `token_mint_1` = USDC, `tick_spacing` = 60. Its owner
//! program (`CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK`) is real
//! Raydium CLMM, confirmed against `raydium::clmm::RAYDIUM_CLMM_PROGRAM_ID`
//! -- **not** CPMM as first assumed this session.
//!
//! Exercises real Token-2022 handling (ATA creation, balance observation
//! bypassing `wallet.token()`'s Token-2022 blindness -- the low-latency
//! `Tokenaccountv1` feed doesn't surface Token-2022 accounts at all),
//! reusing the already real-trade-tested `raydium::clmm` module's account
//! parser (`clmm::parse`) and swap-instruction builder
//! (`clmm::build_swap_ix`) instead of hand-rolled account-layout code.
//!
//! Scope: exactly this one hardcoded pool -- it isn't part of
//! `RaydiumClmm`'s build-time-configured pool list (`raydium_clmm_config
//! ::RAYDIUM_CLMM_POOLS`), since TSLAx isn't in the router's current
//! snapshot (confirmed this session: zero hits for its mint in the
//! generated `router_pools_data.rs`). Ports `RaydiumClmm`'s own
//! tick-array subscribe/confirm-before-use pattern (see that module's
//! `on_account`/`plan_hop` doc comments -- the exact fix for a real,
//! live `NotEnoughTickArrayAccount` failure) using the same vendored, mainnet-verified
//! `solana_clmm_raydium` bitmap-walk math its own private
//! `tick_array_windows` helper uses internally -- duplicated here as a
//! small standalone wrapper since that helper isn't `pub`.

use std::collections::{HashMap, HashSet};

use crate::{
    catscope::witbot::shooter::{Header, Tokenaccountv1},
    graph::{AccountId, Graph, Subscription, SubscriptionQueue, SubscriptionRequest},
    trader::{
        dex::{
            raydium::clmm::{self, RaydiumClmmPool},
            update::Updater,
        },
        pricegraph::TradeRouter,
        types::{SwapParams, TraderError},
    },
    util::{account_id_from_pubkey, pubkey_from_account_id},
    wallet::Wallet,
};
use solana_clmm_raydium::{
    state_helpers::array_start_index_for_tick, tick_array_bit_map::next_initialized_tick_array_start_index,
    tick_array_bit_map::max_tick_in_tickarray_bitmap, PoolTickBitmap, MAX_TICK,
};
use solana_sdk::{clock::Slot, pubkey::Pubkey};

pub const TSLAX_MINT: Pubkey = Pubkey::from_str_const("XsDoVfqeBukxuZHWhdvWHBhgEHjGNst4MLodqsJHzoB");
pub const TSLAX_POOL: Pubkey = Pubkey::from_str_const("HHQUnUbmWLrYzkscDY1C3deEFbGtiGBGoHjpANogmvum");
pub const TOKEN_2022_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

const TICK_ARRAY_SEED: &[u8] = b"tick_array";

/// How many consecutive initialized tick arrays [`initialized_tick_array_start_indices`]
/// walks out to / [`TslaxState::ensure_tick_array_subscriptions`] subscribes
/// to per direction -- enough for a small swap that crosses out of the
/// array containing the current price.
pub const TSLAX_TICK_ARRAY_COUNT: usize = 3;

fn tick_array_pda(pool_pk: &Pubkey, start_index: i32) -> Pubkey {
    Pubkey::find_program_address(
        &[TICK_ARRAY_SEED, pool_pk.as_ref(), &start_index.to_be_bytes()],
        &clmm::RAYDIUM_CLMM_PROGRAM_ID,
    )
    .0
}

/// Real port of the walk `raydium::clmm`'s own private `tick_array_windows`
/// does (see that function's doc comment for the on-chain semantics this
/// mirrors), using the same vendored `solana_clmm_raydium` bitmap-walk
/// math -- duplicated here rather than exposed from that module, see this
/// module's own doc comment. The first entry is always the pool's current
/// window (loaded unconditionally by the on-chain program regardless of
/// its own initialized status, per `tick_array_windows`'s doc comment);
/// subsequent entries are the next real initialized windows found walking
/// in `zero_for_one`'s direction, up to `max_count` total. Returns an
/// empty `Vec` -- a deliberate decline, not a guess -- if the walk runs
/// off the edge of the default bitmap's coverage on a pool whose
/// `tick_spacing` doesn't give it full coverage (never happens for
/// TSLAx's real `tick_spacing = 60`, same math as `tick_array_windows`'s
/// own doc comment: `max_tick_in_tickarray_bitmap(60)` = 1,843,200, far
/// beyond `MAX_TICK`).
fn initialized_tick_array_start_indices(pool: &RaydiumClmmPool, zero_for_one: bool, max_count: usize) -> Vec<i32> {
    let bitmap = PoolTickBitmap::new(pool.tick_array_bitmap);
    let start_0 = array_start_index_for_tick(pool.tick_current, pool.tick_spacing);
    let full_coverage = max_tick_in_tickarray_bitmap(pool.tick_spacing) >= MAX_TICK;

    let mut out = vec![start_0];
    let mut last = start_0;
    while out.len() < max_count {
        let (found, next) = next_initialized_tick_array_start_index(&bitmap, last, pool.tick_spacing, zero_for_one);
        if !found {
            if !full_coverage {
                return Vec::new();
            }
            // Genuine end of the pool's valid price range in this
            // direction, not a coverage gap -- stop here with whatever
            // real windows were already found.
            break;
        }
        out.push(next);
        last = next;
    }
    out
}

/// Live TSLAx pool state: subscribes to exactly [`TSLAX_POOL`] plus its
/// own real, confirmed-existing tick-array accounts -- see this module's
/// doc comment.
pub struct TslaxState {
    pool_id: AccountId,
    o_pool: Option<RaydiumClmmPool>,
    /// Every tick-array start index this bot has ever subscribed to, and
    /// the `AccountId` its PDA derives to.
    m_tick_array_start: HashMap<i32, AccountId>,
    /// Which of `m_tick_array_start`'s accounts have delivered a real
    /// on-chain update -- a phantom bitmap-only claim never produces one
    /// (a pool's bitmap can claim a tick array that was never created, or
    /// has since been closed, on-chain).
    s_confirmed_tick_array: HashSet<AccountId>,
    /// The `tick_current`'s own window start last used to (re)subscribe.
    subscribed_tick_start: Option<i32>,
    tick_array_sub_queue: SubscriptionQueue,
}

impl std::fmt::Debug for TslaxState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TslaxState")
            .field("pool_loaded", &self.o_pool.is_some())
            .field("tick_arrays_known", &self.m_tick_array_start.len())
            .field("tick_arrays_confirmed", &self.s_confirmed_tick_array.len())
            .finish()
    }
}

impl TslaxState {
    pub fn new() -> (Self, Vec<SubscriptionRequest>) {
        let pool_id = account_id_from_pubkey(&TSLAX_POOL);
        let reqs = vec![SubscriptionRequest {
            root: pool_id,
            filter_weight: 0,
            depth: 1,
        }];
        (
            Self {
                pool_id,
                o_pool: None,
                m_tick_array_start: HashMap::new(),
                s_confirmed_tick_array: HashSet::new(),
                subscribed_tick_start: None,
                tick_array_sub_queue: SubscriptionQueue::default(),
            },
            reqs,
        )
    }

    pub fn pool_id(&self) -> AccountId {
        self.pool_id
    }

    /// `None` until a real account update for [`TSLAX_POOL`] has arrived.
    pub fn pool(&self) -> Option<&RaydiumClmmPool> {
        self.o_pool.as_ref()
    }

    /// Walks the pool's bitmap for the initialized tick-array windows a
    /// swap in either direction could need and queues subscriptions for
    /// any not yet subscribed, so each can be confirmed real (see
    /// [`Self::ready_tick_arrays`]) before a swap relies on it. Built on
    /// `raydium::clmm`'s already real-trade-tested pool parser/bitmap data.
    fn ensure_tick_array_subscriptions(&mut self) {
        let Some(pool) = &self.o_pool else { return };
        let start_0 = array_start_index_for_tick(pool.tick_current, pool.tick_spacing);
        if self.subscribed_tick_start == Some(start_0) {
            return;
        }
        let Some(pool_pk) = pubkey_from_account_id(&self.pool_id) else {
            return;
        };
        let mut starts = vec![start_0];
        starts.extend(initialized_tick_array_start_indices(pool, true, TSLAX_TICK_ARRAY_COUNT));
        starts.extend(initialized_tick_array_start_indices(pool, false, TSLAX_TICK_ARRAY_COUNT));
        starts.sort_unstable();
        starts.dedup();
        for start in starts {
            if self.m_tick_array_start.contains_key(&start) {
                continue;
            }
            let ta_id = account_id_from_pubkey(&tick_array_pda(&pool_pk, start));
            self.m_tick_array_start.insert(start, ta_id);
            self.tick_array_sub_queue.push(SubscriptionRequest {
                root: ta_id,
                filter_weight: u32::MAX,
                depth: 1,
            });
        }
        self.subscribed_tick_start = Some(start_0);
    }

    /// The real, confirmed-existing tick-array `AccountId`s to supply for
    /// a swap in `zero_for_one`'s direction, or `None` if any bitmap-
    /// candidate window this swap would need hasn't been confirmed real
    /// yet (a bitmap-claimed array may not actually exist on-chain).
    /// `build_swap_ix` derives its own tick-array list
    /// internally from the same pool data, so this is purely a gate:
    /// callers should only call `build_swap_ix` once this returns `Some`.
    pub fn ready_tick_arrays(&self, zero_for_one: bool) -> Option<Vec<AccountId>> {
        let pool = self.o_pool.as_ref()?;
        let starts = initialized_tick_array_start_indices(pool, zero_for_one, TSLAX_TICK_ARRAY_COUNT);
        if starts.is_empty() {
            return None;
        }
        let mut out = Vec::with_capacity(starts.len());
        for start in starts {
            let id = *self.m_tick_array_start.get(&start)?;
            if !self.s_confirmed_tick_array.contains(&id) {
                return None;
            }
            out.push(id);
        }
        Some(out)
    }

    /// Builds and queues a real `swap_v2` against [`TSLAX_POOL`] via
    /// `raydium::clmm::build_swap_ix` -- real, already-verified account
    /// layout/instruction code, not reimplemented here. Callers must
    /// check [`Self::ready_tick_arrays`] first; this doesn't re-check it
    /// (mirrors `build_swap_ix`'s own contract, which just declines with
    /// `TraderError::MissingConfig` if the pool needs the
    /// `TickArrayBitmapExtension` account -- never true for TSLAx's real
    /// `tick_spacing = 60`, see this module's doc comment).
    pub fn swap(&self, params: &SwapParams, wallet: &mut Wallet) -> Result<(), TraderError> {
        let pool = self.o_pool.as_ref().ok_or(TraderError::PoolNotReady)?;
        clmm::build_swap_ix(self.pool_id, pool, params, wallet)
    }
}

impl Updater for TslaxState {
    fn on_account(&mut self, header: &Header, body: &[u8]) {
        if header.accountid == self.pool_id {
            if let Some(parsed) = clmm::parse(body) {
                self.o_pool = Some(parsed);
            }
            self.ensure_tick_array_subscriptions();
            return;
        }
        if self.m_tick_array_start.values().any(|&id| id == header.accountid) {
            // Real, live-confirmed fix (2026-09-16):
            // merely receiving an update for a subscribed accountid is
            // NOT proof the account exists -- only real, currently
            // rent-exempt, program-owned data counts.
            if header.owner == account_id_from_pubkey(&clmm::RAYDIUM_CLMM_PROGRAM_ID) && header.lamports > 0 {
                self.s_confirmed_tick_array.insert(header.accountid);
            } else {
                self.s_confirmed_tick_array.remove(&header.accountid);
            }
        }
    }

    fn on_token(&mut self, _ta: &Tokenaccountv1) -> bool {
        // TSLAx's own vault balances aren't needed -- swap sizing here is
        // fixed/tiny (see this module's doc comment), not quote-driven.
        false
    }

    fn batch_router(&mut self, _router: &mut TradeRouter) {
        // Not wired into the arbitrage price graph -- testperpv1 calls
        // `swap` directly.
    }

    fn on_tx(&mut self, _ix: &crate::txview::CatscopeInstructionRead<'_>, _slot: &Slot) {}

    // Paced through `tick_array_sub_queue` -- an unbounded per-commit
    // batch here would risk the real `stdio timeout` hangs traced to this
    // pattern earlier in this codebase's history.
    fn flush_pool(&mut self, g: &Graph, max_per_flush: usize) -> Result<(), crate::err::CatscopeGuestError> {
        self.tick_array_sub_queue.flush(g, max_per_flush)?;
        Ok(())
    }
}

#[allow(dead_code)]
fn unused_subscription_type_check(_s: Subscription) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn bitmap_with_bits(bits: &[u32]) -> [u64; 16] {
        let mut words = [0u64; 16];
        for &b in bits {
            words[(b / 64) as usize] |= 1 << (b % 64);
        }
        words
    }

    fn test_pool(tick_current: i32, tick_spacing: u16, bitmap_bits: &[u32]) -> RaydiumClmmPool {
        RaydiumClmmPool {
            token_mint_0: 0,
            token_mint_1: 0,
            token_vault_0: 0,
            token_vault_1: 0,
            amm_config: 0,
            observation_key: 0,
            mint_decimals_0: 8,
            mint_decimals_1: 6,
            tick_spacing,
            liquidity: 1,
            sqrt_price_x64: 0,
            tick_current,
            tick_array_bitmap: bitmap_with_bits(bitmap_bits),
            reserve_0: 0,
            reserve_1: 0,
        }
    }

    #[test]
    fn initialized_tick_array_start_indices_includes_current_window_first() {
        // bit 512 = start index 0, matches tick_current=0's own window --
        // set, so it's returned as-is with no need to search further.
        let pool = test_pool(0, 60, &[512]);
        let starts = initialized_tick_array_start_indices(&pool, true, 3);
        assert_eq!(starts, vec![0]);
    }

    #[test]
    fn initialized_tick_array_start_indices_walks_outward_when_current_empty() {
        let multiplier = 60i32 * 60; // tick_spacing * TICK_ARRAY_SIZE
        // tick_current=100 -> own window start=0 -> bit 512, left UNSET.
        // Real initialized arrays one below (bit 511) and one above
        // (bit 513).
        let pool = test_pool(100, 60, &[511, 513]);
        let ups = initialized_tick_array_start_indices(&pool, false, 3);
        assert_eq!(ups, vec![0, multiplier]);
        let downs = initialized_tick_array_start_indices(&pool, true, 3);
        assert_eq!(downs, vec![0, -multiplier]);
    }

    #[test]
    fn tick_array_pda_matches_real_program_id() {
        // Real PDA derivation must use Raydium CLMM's program, not any
        // other -- a live mismatch here would silently
        // reference the wrong account on-chain.
        let pool_pk = TSLAX_POOL;
        let addr = tick_array_pda(&pool_pk, 0);
        let (expected, _) = Pubkey::find_program_address(
            &[TICK_ARRAY_SEED, pool_pk.as_ref(), &0i32.to_be_bytes()],
            &clmm::RAYDIUM_CLMM_PROGRAM_ID,
        );
        assert_eq!(addr, expected);
    }
}
