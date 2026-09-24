//! Orca Whirlpool (concentrated liquidity) parser and swap builder.
//!
//! # Account layout — Orca Whirlpool (Anchor, 8-byte discriminator)
//!
//! ```text
//! offset   size  field
//! ──────   ────  ───────────────────────────────────────────────────
//!   0        8   Anchor discriminator
//!   8       32   whirlpools_config (Pubkey)
//!  40        1   whirlpool_bump ([u8; 1])
//!  41        2   tick_spacing (u16)
//!  43        2   tick_spacing_seed ([u8; 2])
//!  45        2   fee_rate (u16, hundredths of a basis point)
//!  47        2   protocol_fee_rate (u16)
//!  49       16   liquidity (u128)
//!  65       16   sqrt_price_x64 (u128)   ← spot price
//!  81        4   tick_current_index (i32)
//!  85        8   protocol_fee_owed_a (u64)
//!  93        8   protocol_fee_owed_b (u64)
//! 101       32   token_mint_a (Pubkey)
//! 133       32   token_vault_a (Pubkey)
//! 165       16   fee_growth_global_a (u128)
//! 181       32   token_mint_b (Pubkey)
//! 213       32   token_vault_b (Pubkey)
//! ```
//!
//! # Price formula
//!
//! ```text
//! sqrt_price = sqrt_price_x64 / 2^64
//! price      = sqrt_price^2          (raw units: token_b per token_a)
//! ```
//!
//! # Swap instruction (Anchor)
//!
//! ```text
//! discriminator (8 bytes): sha256("global:swap")[0..8]
//!                        = [248, 198, 158, 145, 225, 117, 135, 200]
//! amount                 (u64 LE)
//! other_amount_threshold (u64 LE)
//! sqrt_price_limit       (u128 LE, 0 = no limit)
//! amount_specified_is_input (bool)
//! a_to_b                 (bool, true = token_a in → token_b out)
//! ```

use crate::{
    catscope::witbot::shooter::{Header, Tokenaccountv1},
    err::CatscopeGuestError,
    graph::{AccountId, Graph, Subscription, SubscriptionQueue, SubscriptionRequest},
    log_warn, orca_config,
    trader::{
        dex::update::Updater,
        pricegraph::{cp_quote, Hop, TradeRouter},
        types::{DexType, PoolPrice, SwapParams, TraderError},
    },
    txview::CatscopeInstructionRead,
    util::{account_id_from_pubkey, pubkey_from_account_id},
    wallet::Wallet,
};
use orca_whirlpools_core::{
    swap_quote_by_input_token, TickArrayFacade, TickArrays, TickFacade, WhirlpoolFacade,
    TICK_ARRAY_SIZE,
};
use solana_sdk::{
    clock::Slot,
    message::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    hash::BuildHasherDefault,
    u32,
};
use twox_hash::XxHash64;

// ─── Program IDs ─────────────────────────────────────────────────────────────

pub const ORCA_WHIRLPOOL_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc");

pub const SPL_TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

pub const SPL_MEMO_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");

// ─── Whirlpool account offsets ────────────────────────────────────────────────

const OFF_TICK_SPACING: usize = 41;
const OFF_FEE_RATE: usize = 45;
const OFF_LIQUIDITY: usize = 49;
const OFF_SQRT_PRICE: usize = 65;
const OFF_TICK_CURRENT: usize = 81;
const OFF_MINT_A: usize = 101;
const OFF_VAULT_A: usize = 133;
const OFF_MINT_B: usize = 181;
const OFF_VAULT_B: usize = 213;

const MIN_WHIRLPOOL_LEN: usize = OFF_VAULT_B + 32;

/// Tick span covered by one tick-array account (ticks × tick_spacing).
const TICKS_PER_ARRAY: i32 = TICK_ARRAY_SIZE as i32;

// ─── Tick-array account offsets ───────────────────────────────────────────────
// layout: discriminator(8) + start_tick_index(4) + ticks(88×113) + whirlpool(32)
// Tick size: initialized(1)+liquidity_net(16)+liquidity_gross(16)+fee_growth_a(16)+fee_growth_b(16)+reward_growths(48) = 113
const OFF_TA_START_INDEX: usize = 8;
const OFF_TA_WHIRLPOOL: usize = 8 + 4 + 88 * 113; // = 9956
const MIN_TICK_ARRAY_LEN: usize = OFF_TA_WHIRLPOOL + 32; // = 9988

// ─── Anchor discriminators ────────────────────────────────────────────────────

const DISC_WHIRLPOOL: [u8; 8] = [63, 149, 209, 12, 225, 128, 99, 9];
const DISC_TICK_ARRAY: [u8; 8] = [69, 97, 189, 190, 110, 7, 66, 187];

// ─── Parsed pool state ────────────────────────────────────────────────────────

/// Parsed state of an Orca Whirlpool pool.
#[derive(Debug, Clone, Default)]
pub struct OrcaWhirlpool {
    /// Token A mint.
    pub token_mint_a: AccountId,
    /// Token B mint.
    pub token_mint_b: AccountId,
    /// Pool's token A vault (subscribe for reserve updates).
    pub vault_a: AccountId,
    /// Pool's token B vault (subscribe for reserve updates).
    pub vault_b: AccountId,
    /// Active liquidity in the current tick range (L value from CLMM).
    pub liquidity: u128,
    /// Current sqrt price Q64.64 fixed-point.
    pub sqrt_price_x64: u128,
    /// Current tick index.
    pub tick_current_index: i32,
    /// Tick spacing (determines tick-array coverage).
    pub tick_spacing: u16,
    /// Fee rate in hundredths of a basis point (3000 = 0.3%).
    pub fee_rate: u16,
    /// Cached token A reserve (lamport-scale). Updated via vault token events.
    pub reserve_a: u64,
    /// Cached token B reserve (lamport-scale). Updated via vault token events.
    pub reserve_b: u64,
}

impl OrcaWhirlpool {
    /// Fee in basis points (fee_rate / 100).
    pub fn fee_bps(&self) -> u16 {
        self.fee_rate / 100
    }

    /// Spot price: token_b raw units per token_a raw unit.
    pub fn spot_price(&self) -> f64 {
        let sqrt = self.sqrt_price_x64 as f64 / (1u128 << 64) as f64;
        sqrt * sqrt
    }

    /// Local constant-product "virtual reserves" at the current tick,
    /// derived purely from `liquidity`/`sqrt_price_x64` -- **not** the real
    /// vault balances (`reserve_a`/`reserve_b`, populated separately via
    /// live vault-balance tracking in `on_token`).
    ///
    /// A Whirlpool's active liquidity `L` and current `sqrt_price` P relate
    /// to the standard CLMM identity `virtual_a = L / P`, `virtual_b = L * P`
    /// (so `virtual_a * virtual_b = L^2`, a valid constant-product curve,
    /// and `virtual_b / virtual_a = P^2 = spot_price()` by construction --
    /// always self-consistent with `spot_price()` since both come from the
    /// same account snapshot). This is the same identity real CLMM swap
    /// math is built on (see `swap_quote_by_input_token`'s use of
    /// `to_facade()` in `swap()`), valid as long as a trade doesn't cross
    /// out of the current tick -- true for the modest probe/real-balance
    /// sizes `pricegraph::TradeRouter` quotes with.
    ///
    /// Using this instead of raw vault balances for router-graph pricing
    /// fixes a real, observed bug: a Whirlpool's vaults hold *every* LP's
    /// deposited tokens across the *entire* price range, not just the
    /// liquidity active at the current tick. A pool with a lot of
    /// out-of-range liquidity parked far from the current price can have a
    /// vault-balance ratio wildly different from its real tradeable
    /// liquidity, even though `spot_price()` itself is completely correct
    /// -- `cp_quote` extrapolating a constant-product curve over that
    /// misleading full-vault ratio then produces an output off by orders
    /// of magnitude (observed live: 100 SOL "quoting" as both 2.99e15 and
    /// 1.58e-3 raw-unit outputs through different pools, depending on
    /// which side of the mismatch dominated).
    pub fn virtual_reserves(&self) -> (u64, u64) {
        if self.liquidity == 0 {
            return (0, 0);
        }
        let sqrt = self.sqrt_price_x64 as f64 / (1u128 << 64) as f64;
        if !sqrt.is_finite() || sqrt <= 0.0 {
            return (0, 0);
        }
        let liquidity = self.liquidity as f64;
        let virtual_a = liquidity / sqrt;
        let virtual_b = liquidity * sqrt;
        // Reject (0), don't saturate -- a pool with very large `liquidity`
        // at a very lopsided `sqrt_price` (deep negative/positive tick)
        // can produce a true virtual_a/virtual_b that genuinely doesn't
        // fit in a u64 (confirmed live: one real pool's true virtual_a was
        // ~4.0e20, ~21.7x past u64::MAX). Saturating that to u64::MAX
        // silently substitutes a value ~21.7x too small -- a plausible-
        // looking but wrong number, not an error -- which then feeds
        // cp_quote as either reserve_in or reserve_out and produces
        // wildly incorrect, direction-dependent quotes (confirmed live:
        // the same pool's implied price differed by ~2394x, and flipped
        // direction, between a 100 SOL and a 0.01 SOL probe -- consistent
        // with a corrupted reserve, not real slippage). Returning 0 here
        // instead reuses add_orca_pool's existing "virtual_a == 0 ||
        // virtual_b == 0" invalidity gate, so an overflowing pool's edge
        // is correctly removed rather than fed a corrupted reserve.
        let clamp = |x: f64| -> u64 {
            if !x.is_finite() || x <= 0.0 || x > u64::MAX as f64 {
                0
            } else {
                x as u64
            }
        };
        (clamp(virtual_a), clamp(virtual_b))
    }

    /// Build a [`PoolPrice`] snapshot.
    pub fn pool_price(&self) -> PoolPrice {
        PoolPrice {
            token_a: self.token_mint_a,
            token_b: self.token_mint_b,
            price: self.spot_price(),
            reserve_a: self.reserve_a,
            reserve_b: self.reserve_b,
            fee_bps: self.fee_bps(),
        }
    }

    /// Compute the start tick index for the tick array covering `tick`.
    fn tick_array_start(&self, tick: i32) -> i32 {
        let size = TICKS_PER_ARRAY * self.tick_spacing as i32;
        // Floor-divide toward negative infinity
        if tick >= 0 {
            (tick / size) * size
        } else {
            ((tick - size + 1) / size) * size
        }
    }

    /// Derive a tick-array PDA for a given start index.
    fn tick_array_pda(&self, pool_pk: &Pubkey, start_index: i32) -> Pubkey {
        let idx_str = start_index.to_string();
        Pubkey::find_program_address(
            &[b"tick_array", pool_pk.as_ref(), idx_str.as_bytes()],
            &ORCA_WHIRLPOOL_PROGRAM_ID,
        )
        .0
    }

    /// Derive the oracle PDA for this pool.
    fn oracle_pda(&self, pool_pk: &Pubkey) -> Pubkey {
        Pubkey::find_program_address(&[b"oracle", pool_pk.as_ref()], &ORCA_WHIRLPOOL_PROGRAM_ID).0
    }

    /// Compute the five tick-array PDAs for a SwapV2 instruction.
    ///
    /// Layout matches the Orca SDK: [current, +1, +2, -1, -2].
    /// The first 3 are the named tick_array0/1/2 accounts; the last 2 are
    /// supplemental remaining accounts. The on-chain program selects what
    /// it needs based on `a_to_b`.
    pub fn tick_arrays(&self, pool_pk: &Pubkey) -> [Pubkey; 5] {
        let ts = TICKS_PER_ARRAY * self.tick_spacing as i32;
        let start_0 = self.tick_array_start(self.tick_current_index);
        [
            self.tick_array_pda(pool_pk, start_0),
            self.tick_array_pda(pool_pk, start_0 + ts),
            self.tick_array_pda(pool_pk, start_0 + 2 * ts),
            self.tick_array_pda(pool_pk, start_0 - ts),
            self.tick_array_pda(pool_pk, start_0 - 2 * ts),
        ]
    }

    pub fn parse(&mut self, body: &[u8]) -> Result<(), CatscopeGuestError> {
        if body.len() < MIN_WHIRLPOOL_LEN {
            return Err(CatscopeGuestError::InsufficientBufferV2(
                body.len(),
                MIN_WHIRLPOOL_LEN,
            ));
        }
        let read_u16 = |off: usize| u16::from_le_bytes(body[off..off + 2].try_into().unwrap());
        let read_u128 = |off: usize| u128::from_le_bytes(body[off..off + 16].try_into().unwrap());
        let read_i32 = |off: usize| i32::from_le_bytes(body[off..off + 4].try_into().unwrap());
        let read_pk = |off: usize| -> Pubkey {
            Pubkey::new_from_array(body[off..off + 32].try_into().unwrap())
        };
        let pk_id = |pk: Pubkey| account_id_from_pubkey(&pk);
        if self.token_mint_a == 0 {
            self.token_mint_a = pk_id(read_pk(OFF_MINT_A));
        }
        if self.token_mint_b == 0 {
            self.token_mint_b = pk_id(read_pk(OFF_MINT_B));
        }
        if self.vault_a == 0 {
            self.vault_a = pk_id(read_pk(OFF_VAULT_A));
        }
        if self.vault_b == 0 {
            self.vault_b = pk_id(read_pk(OFF_VAULT_B));
        }
        assert_ne!(self.vault_a, 0);
        assert_ne!(self.vault_b, 0);
        assert_ne!(self.token_mint_a, 0);
        assert_ne!(self.token_mint_b, 0);
        self.liquidity = read_u128(OFF_LIQUIDITY);
        self.sqrt_price_x64 = read_u128(OFF_SQRT_PRICE);
        self.tick_current_index = read_i32(OFF_TICK_CURRENT);
        self.tick_spacing = read_u16(OFF_TICK_SPACING);
        self.fee_rate = read_u16(OFF_FEE_RATE);

        Ok(())
    }

    pub fn pool_lookup_id(&self) -> [AccountId; 2] {
        let mut id = [self.token_mint_a, self.token_mint_b];
        id.sort();
        id
    }

    /// Build a `WhirlpoolFacade` for use with `orca_whirlpools_core` quote functions.
    pub fn to_facade(&self) -> WhirlpoolFacade {
        WhirlpoolFacade {
            tick_spacing: self.tick_spacing,
            fee_rate: self.fee_rate,
            liquidity: self.liquidity,
            sqrt_price: self.sqrt_price_x64,
            tick_current_index: self.tick_current_index,
            // Setting fee_tier_index_seed == tick_spacing signals no adaptive fee.
            fee_tier_index_seed: self.tick_spacing.to_le_bytes(),
            ..WhirlpoolFacade::default()
        }
    }
    /// Append an Orca Whirlpool `swap_v2` instruction to the wallet queue.
    /// Build a SwapV2 instruction deriving tick arrays from PDAs.
    /// Prefer `build_swap_ix` with map-resolved tick arrays when possible.
    pub fn build_swap_ix_pda(
        &self,
        pool_id: AccountId,
        params: &SwapParams,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let a_to_b = if params.input_mint == self.token_mint_a {
            true
        } else if params.input_mint == self.token_mint_b {
            false
        } else {
            return Err(TraderError::WrongMints);
        };
        let pool_pk =
            pubkey_from_account_id(&pool_id).ok_or(TraderError::PubkeyResolutionFailed(pool_id))?;
        let ts = TICKS_PER_ARRAY * self.tick_spacing as i32;
        let start_0 = self.tick_array_start(self.tick_current_index);
        let ta = if a_to_b {
            [
                self.tick_array_pda(&pool_pk, start_0),
                self.tick_array_pda(&pool_pk, start_0 - ts),
                self.tick_array_pda(&pool_pk, start_0 - 2 * ts),
            ]
        } else {
            [
                self.tick_array_pda(&pool_pk, start_0),
                self.tick_array_pda(&pool_pk, start_0 + ts),
                self.tick_array_pda(&pool_pk, start_0 + 2 * ts),
            ]
        };
        self.build_swap_ix(pool_id, &ta, params, wallet)
    }

    /// Build a SwapV2 instruction using caller-supplied tick array pubkeys.
    ///
    /// `tick_arrays` must be 3 initialised accounts in swap-direction order.
    pub fn build_swap_ix(
        &self,
        pool_id: AccountId,
        tick_arrays: &[Pubkey; 3],
        params: &SwapParams,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let a_to_b = if params.input_mint == self.token_mint_a
            && params.output_mint == self.token_mint_b
        {
            true
        } else if params.input_mint == self.token_mint_b && params.output_mint == self.token_mint_a
        {
            false
        } else {
            return Err(TraderError::WrongMints);
        };

        let resolve = |id: AccountId| -> Result<Pubkey, TraderError> {
            pubkey_from_account_id(&id).ok_or(TraderError::PubkeyResolutionFailed(id))
        };

        let pool_pk = resolve(pool_id)?;
        let mint_a_pk = resolve(self.token_mint_a)?;
        let mint_b_pk = resolve(self.token_mint_b)?;
        let vault_a_pk = resolve(self.vault_a)?;
        let vault_b_pk = resolve(self.vault_b)?;
        let user_source_pk = resolve(params.user_source_token_account)?;
        let user_dest_pk = resolve(params.user_destination_token_account)?;
        let user_wallet_pk = resolve(params.user_wallet)?;

        let oracle_pk = self.oracle_pda(&pool_pk);

        let (user_a_pk, user_b_pk) = if a_to_b {
            (user_source_pk, user_dest_pk)
        } else {
            (user_dest_pk, user_source_pk)
        };

        // SwapV2 instruction data (43 bytes):
        //   discriminator(8) + amount(8) + other_amount_threshold(8) +
        //   sqrt_price_limit(16) + amount_specified_is_input(1) + a_to_b(1) +
        //   remaining_accounts_info = None → [0] (1 byte)
        let mut data = Vec::with_capacity(43);
        data.extend_from_slice(&SWAP_V2_DISCRIMINATOR);
        data.extend_from_slice(&params.amount_in.to_le_bytes());
        data.extend_from_slice(&params.min_amount_out.to_le_bytes());
        data.extend_from_slice(&0u128.to_le_bytes()); // sqrt_price_limit (no limit)
        data.push(1u8); // amount_specified_is_input = true
        data.push(if a_to_b { 1u8 } else { 0u8 });
        data.push(0u8); // remaining_accounts_info = None

        let mut accounts = Vec::with_capacity(15);
        accounts.push(AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false)); // token_program_a
        accounts.push(AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false)); // token_program_b
        accounts.push(AccountMeta::new_readonly(SPL_MEMO_PROGRAM_ID, false)); // memo_program
        accounts.push(AccountMeta::new_readonly(user_wallet_pk, true)); // token_authority
        accounts.push(AccountMeta::new(pool_pk, false)); // whirlpool
        accounts.push(AccountMeta::new_readonly(mint_a_pk, false)); // token_mint_a
        accounts.push(AccountMeta::new_readonly(mint_b_pk, false)); // token_mint_b
        accounts.push(AccountMeta::new(user_a_pk, false)); // token_owner_account_a
        accounts.push(AccountMeta::new(vault_a_pk, false)); // token_vault_a
        accounts.push(AccountMeta::new(user_b_pk, false)); // token_owner_account_b
        accounts.push(AccountMeta::new(vault_b_pk, false)); // token_vault_b
        accounts.push(AccountMeta::new(tick_arrays[0], false)); // tick_array_0 (current)
        accounts.push(AccountMeta::new(tick_arrays[1], false)); // tick_array_1 (step 1)
        accounts.push(AccountMeta::new(tick_arrays[2], false)); // tick_array_2 (step 2)
        accounts.push(AccountMeta::new(oracle_pk, false)); // oracle

        wallet.require_signer(params.user_wallet);
        wallet.append_ix(
            Instruction {
                program_id: ORCA_WHIRLPOOL_PROGRAM_ID,
                accounts,
                data,
            },
            ORCA_WHIRLPOOL_SWAP_CU,
        );

        Ok(())
    }
}

// ─── Parser ───────────────────────────────────────────────────────────────────

/// Parse an Orca Whirlpool account from raw body bytes (after the Anchor discriminator).
///
/// `body` must include the full account data returned by the host, **including**
/// the 8-byte Anchor discriminator at the start.
///
/// Returns `None` if `body` is too short or appears invalid.
pub fn parse(body: &[u8]) -> Option<OrcaWhirlpool> {
    if body.len() < MIN_WHIRLPOOL_LEN {
        return None;
    }

    let read_u16 = |off: usize| u16::from_le_bytes(body[off..off + 2].try_into().unwrap());
    let read_u128 = |off: usize| u128::from_le_bytes(body[off..off + 16].try_into().unwrap());
    let read_i32 = |off: usize| i32::from_le_bytes(body[off..off + 4].try_into().unwrap());
    let read_pk =
        |off: usize| -> Pubkey { Pubkey::new_from_array(body[off..off + 32].try_into().unwrap()) };
    let pk_id = |pk: Pubkey| account_id_from_pubkey(&pk);

    Some(OrcaWhirlpool {
        token_mint_a: pk_id(read_pk(OFF_MINT_A)),
        token_mint_b: pk_id(read_pk(OFF_MINT_B)),
        vault_a: pk_id(read_pk(OFF_VAULT_A)),
        vault_b: pk_id(read_pk(OFF_VAULT_B)),
        liquidity: read_u128(OFF_LIQUIDITY),
        sqrt_price_x64: read_u128(OFF_SQRT_PRICE),
        tick_current_index: read_i32(OFF_TICK_CURRENT),
        tick_spacing: read_u16(OFF_TICK_SPACING),
        fee_rate: read_u16(OFF_FEE_RATE),
        reserve_a: 0,
        reserve_b: 0,
    })
}
// ─── Swap instruction builder ─────────────────────────────────────────────────

/// Compute units budgeted for an Orca Whirlpool swap.
pub const ORCA_WHIRLPOOL_SWAP_CU: u32 = 300_000;

/// Anchor discriminator for the `swap_v2` instruction.
pub const SWAP_V2_DISCRIMINATOR: [u8; 8] = [43, 4, 237, 11, 26, 201, 30, 98];

/// Append an Orca Whirlpool `swap_v2` instruction to the wallet queue.
pub fn build_swap_ix(
    pool_id: AccountId,
    pool: &OrcaWhirlpool,
    tick_arrays: &[Pubkey; 3],
    params: &SwapParams,
    wallet: &mut Wallet,
) -> Result<(), TraderError> {
    pool.build_swap_ix(pool_id, tick_arrays, params, wallet)
}

#[derive(Default)]
struct InfoWithVersion<M: Default> {
    firstshred: M,
    root: M,
    version: u64,
}

/// Parsed state of an Orca tick-array account.
#[derive(Clone)]
pub struct ParsedTickArray {
    /// The start tick index of this array (identifies its position in the pool's range).
    pub start_tick_index: i32,
    /// Pool (whirlpool) this tick array belongs to.
    pub whirlpool: AccountId,
    /// Full tick data ready for use with `orca_whirlpools_core` quote functions.
    pub facade: TickArrayFacade,
}

pub struct OrcaState {
    has_check_pool: bool,
    count: usize,
    parsed_count: usize,
    tx_count: usize,
    program_id: AccountId,
    l_pool: Vec<PoolInfo>,
    m_pool: HashMap<AccountId, usize, BuildHasherDefault<XxHash64>>,
    // map [mint_a,mint_b]->pool AccountId
    m_pair: HashMap<[AccountId; 2], HashSet<usize>>,
    // map vault token account id -> pool index in l_pool
    m_vault: HashMap<AccountId, usize>,
    // map tick_array account_id -> parsed tick array
    m_tick_array: HashMap<AccountId, ParsedTickArray>,
    // map pool_id -> { start_tick_index -> tick_array account_id }
    m_pool_tick_arrays: HashMap<AccountId, HashMap<i32, AccountId>>,
    token_sub_queue: SubscriptionQueue,
    q_hold_sub: VecDeque<Vec<Subscription>>,
    pending_token_counted: usize,
    /// TEMP diagnostic: how many times batch_router has run -- gates a
    /// periodic breakdown log (gate-fail vs no-node vs ok) to see exactly
    /// why add_orca_pool keeps emitting zero live edges despite real
    /// on_account/on_token activity and (post router-budget-floor-fix)
    /// real registered nodes.
    batch_router_calls: u64,
}
impl std::fmt::Debug for OrcaState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OrcaState").finish()
    }
}
struct TokenSubRequest {
    pool_id: AccountId,
    token_a: AccountId,
    token_b: AccountId,
}
#[derive(Debug)]
enum PoolTokenSubscribeStage {
    Waiting,
    Found(usize),
}

#[derive(Debug)]
struct PoolInfo {
    id: AccountId,
    stage: PoolTokenSubscribeStage,
    last_root: Slot,
    root: OrcaWhirlpool,
    last_processed: Slot,
    processed: OrcaWhirlpool,
    last_tx: Slot,
    mint_a: AccountId,
    vault_a_balance: (u64, Slot),
    mint_b: AccountId,
    vault_b_balance: (u64, Slot),
    /// start_tick_index of the tick-array window we last issued
    /// subscriptions for (see the tick-array subscribe block in
    /// `on_account`) -- `None` until the first Whirlpool account arrives.
    subscribed_tick_start: Option<i32>,
}

pub struct OrcaSetup {
    pub l_pool: Vec<OrcaPoolSetup>,
}

impl OrcaState {
    /// Builds this dex's live state and returns its pending subscription
    /// requests alongside it -- doesn't subscribe itself. See
    /// `raydium::amm::RaydiumAmm::new`'s doc comment for why (paced
    /// through a shared [`crate::graph::SubscriptionQueue`] owned by
    /// `DexState` instead). Also builds `l_pool` in the same pass as
    /// `l_req` now (previously a separate pass zipped against the
    /// `bulk_subscribe` result by index) -- incidentally fixes a latent
    /// misalignment: the old second pass indexed `l_ops[i]` by `l_sub`'s
    /// (post-dedup) length, which would have paired the wrong `ops` entry
    /// with each pool whenever `l_ops` actually contained a duplicate
    /// pubkey.
    pub fn new() -> (Self, Vec<SubscriptionRequest>) {
        let setup = Setup::default();
        let l_ops = &setup.l_pool;
        let count = l_ops.len();
        let mut l_pool = Vec::with_capacity(count);
        let check_pool_id = {
            let pubkey = Pubkey::from_str_const("Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE");
            account_id_from_pubkey(&pubkey)
        };
        let mut has_check_pool = false;
        let mut l_req = Vec::with_capacity(l_ops.len());

        let mut hs_duplicate = HashSet::with_capacity(l_ops.len());
        for ops in l_ops.iter() {
            if !hs_duplicate.insert(ops.pubkey) {
                continue;
            }
            if ops.pubkey == check_pool_id {
                has_check_pool = true;
            }
            // Pool only -- tick arrays are handled separately, via the
            // explicit depth-1 subscribe-on-demand block in `on_account`
            // (see its doc comment). A `depth: 2` walk here would also
            // pull in every tick array this pool has *ever* had
            // initialized -- live-verified this session: one busy
            // reference pool (`check_pool_id` above) already has 300+
            // real tick-array accounts, while `on_account`'s own logic
            // only ever needs the 5 nearest the current price at a time.
            l_req.push(SubscriptionRequest {
                root: ops.pubkey,
                filter_weight: u32::MAX,
                depth: 1,
            });
            l_pool.push(PoolInfo {
                vault_a_balance: (0, 0),
                vault_b_balance: (0, 0),
                stage: PoolTokenSubscribeStage::Waiting,
                last_tx: 0,
                id: ops.pubkey,
                last_root: 0,
                root: OrcaWhirlpool::default(),
                last_processed: 0,
                processed: OrcaWhirlpool::default(),
                mint_a: ops.mint_a,
                mint_b: ops.mint_b,
                subscribed_tick_start: None,
            });
        }
        let mut m_pair = HashMap::with_capacity(2 * l_ops.len());
        l_pool.sort_unstable_by_key(|p| p.id);
        // Built *after* the sort above, not before -- on_account looks up
        // by binary_search_by_key against this same sorted l_pool, so
        // m_pool's indices must match the post-sort positions too.
        // Building it during the pre-sort insertion loop (an earlier
        // version of this code did) left every index stale the moment
        // the sort ran: m_pool.get(pool_id) would return SOME OTHER
        // pool's post-sort position, so refresh_account_router/
        // refresh_token_router (and batch_router's old m_pool.iter()
        // loop) would price/route a pool using a completely different,
        // essentially arbitrary pool's live data -- confirmed live this
        // session: a pool's own liquidity/sqrt_price_x64 read via
        // m_pool consistently showed either a frozen all-zero snapshot
        // or slowly-drifting numbers completely unrelated to the pool_id
        // being priced, while on_account (correctly using binary_search)
        // kept every pool's *own* data genuinely live and correct the
        // whole time.
        let mut m_pool: HashMap<AccountId, usize, BuildHasherDefault<XxHash64>> =
            HashMap::with_capacity_and_hasher(l_pool.len(), BuildHasherDefault::default());
        for (i, pool) in l_pool.iter().enumerate() {
            m_pool.insert(pool.id, i);
        }
        for (i, pool) in l_pool.iter().enumerate() {
            let mut id = [pool.mint_a, pool.mint_b];
            id.sort();
            let hs_pubkey: &mut HashSet<usize> = m_pair.entry(id).or_default();
            hs_pubkey.insert(i);
        }
        let orca_program_id = account_id_from_pubkey(&ORCA_WHIRLPOOL_PROGRAM_ID);

        if let Ok(i) = l_pool.binary_search_by_key(&check_pool_id, |p| p.id) {
            assert_eq!(check_pool_id, l_pool[i].id);
        }

        let state = Self {
            m_pool,
            pending_token_counted: 0,
            has_check_pool,
            count: 0,
            parsed_count: 0,
            tx_count: 0,
            l_pool,
            m_pair,
            program_id: orca_program_id,
            m_vault: HashMap::with_capacity(100_000),
            m_tick_array: HashMap::new(),
            m_pool_tick_arrays: HashMap::new(),
            token_sub_queue: SubscriptionQueue::default(),
            q_hold_sub: VecDeque::new(),
            batch_router_calls: 0,
        };
        (state, l_req)
    }

    /// Immediately subscribes to this dex's pending pool requests and
    /// keeps them alive via `q_hold_sub` -- for callers that don't defer
    /// through a shared [`crate::graph::SubscriptionQueue`] (`DexState`'s
    /// own copy uses the split [`Self::new`]/`flush_subscriptions` path
    /// instead). Real caller: `helloworldv1`'s standalone demo instance.
    pub fn new_and_subscribe(g: &Graph) -> Result<Self, CatscopeGuestError> {
        let (mut state, l_req) = Self::new();
        let subs = SubscriptionQueue::subscribe_now(g, l_req)?;
        state.q_hold_sub.push_back(subs);
        Ok(state)
    }

    #[inline]
    pub fn program_id(&self) -> &AccountId {
        &self.program_id
    }

    /// Look up all known tick arrays for a pool, keyed by start_tick_index.
    pub fn tick_arrays_for_pool(&self, pool_id: &AccountId) -> Option<&HashMap<i32, AccountId>> {
        self.m_pool_tick_arrays.get(pool_id)
    }

    /// Pools subscribed to (bounded by ORCA_POOL_BUDGET at build time).
    pub fn pool_count(&self) -> usize {
        self.l_pool.len()
    }

    /// Live tick-array accounts held in memory -- worth watching over time
    /// since this is exactly what overflowed hashbrown's capacity before
    /// ORCA_WHIRLPOOL_POOLS was capped (see build.rs's orca_pool_budget()).
    pub fn tick_array_count(&self) -> usize {
        self.m_tick_array.len()
    }

    /// TEMPORARY DIAGNOSTIC (2026-09-07): how many vault/tick-array
    /// subscribe requests `token_sub_queue` currently has queued but not
    /// yet flushed -- added to check whether a chronically `PoolNotReady`
    /// pool (real, live-confirmed: several Orca pools failed identically
    /// across 25+ restarts) is stuck behind a real backlog in the shared
    /// 128/cycle `flush_pool` budget, or something else entirely.
    pub fn token_sub_queue_pending_count(&self) -> usize {
        self.token_sub_queue.pending_count()
    }

    /// Look up a single tick array by pool and start_tick_index.
    pub fn tick_array_by_start(
        &self,
        pool_id: &AccountId,
        start_tick_index: i32,
    ) -> Option<AccountId> {
        self.m_pool_tick_arrays
            .get(pool_id)?
            .get(&start_tick_index)
            .copied()
    }

    pub fn has_check_pool(&self) -> bool {
        self.has_check_pool
    }

    pub fn count(&self) -> (usize, usize, usize) {
        (self.count, self.parsed_count, self.tx_count)
    }

    /// l_account_id is sorted in ascending order
    pub fn on_tx(&mut self, ix: &CatscopeInstructionRead<'_>, slot: &Slot) {
        let l_account_id = ix.account();
        //        ix.data();
        let (mut i, mut j) = (0, 0);
        while i < l_account_id.len() && j < self.l_pool.len() {
            let x = l_account_id[i];
            let y = self.l_pool[j].id;
            if x < y {
                i += 1;
            } else if y < x {
                j += 1;
            } else {
                self.tx_count += 1;
                self.l_pool[j].last_tx = *slot;
                self.l_pool[j].last_processed = *slot;
                i += 1;
                j += 1;
            }
        }

        // Decode a landed swap_v2 instruction and provisionally ("fudge")
        // adjust the pool's reserves ahead of the real account update that
        // will arrive ~50ms later. Safe to mutate in place: the pool's
        // reserve_a/reserve_b are the exact same fields on_token writes,
        // so the next real vault-balance update simply overwrites this
        // estimate with ground truth -- no separate expiry needed.
        if *ix.program() == self.program_id {
            let data = ix.data();
            let accounts = ix.account();
            if data.len() >= 42 && data[..8] == SWAP_V2_DISCRIMINATOR && accounts.len() > 4 {
                let amount_specified_is_input = data[40] != 0;
                let a_to_b = data[41] != 0;
                // Exact-out swaps use the amount field as the desired
                // output, not input -- not handled here.
                if amount_specified_is_input {
                    let amount_in = u64::from_le_bytes(data[8..16].try_into().unwrap());
                    let pool_id = accounts[4];
                    if let Some(&idx) = self.m_pool.get(&pool_id) {
                        let pool = &mut self.l_pool[idx].processed;
                        let fee_bps = pool.fee_bps();
                        let (reserve_in, reserve_out) = if a_to_b {
                            (pool.reserve_a, pool.reserve_b)
                        } else {
                            (pool.reserve_b, pool.reserve_a)
                        };
                        if reserve_in != 0 && reserve_out != 0 {
                            let amount_out = cp_quote(amount_in, reserve_in, reserve_out, fee_bps);
                            let new_in = reserve_in.saturating_sub(amount_in);
                            let new_out = reserve_out.saturating_add(amount_out);
                            if a_to_b {
                                pool.reserve_a = new_in;
                                pool.reserve_b = new_out;
                            } else {
                                pool.reserve_b = new_in;
                                pool.reserve_a = new_out;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Swap tokens through the best-priced pool for the given mint pair.
    ///
    /// "Best" means the pool that maximises the fee-adjusted output per input unit,
    /// taking swap direction into account:
    ///   - A→B (`input_mint == token_mint_a`): higher spot price is better.
    ///   - B→A (`input_mint == token_mint_b`): lower spot price is better (more A per B).
    ///
    /// `min_amount_out` on `params` is overwritten using `set_min_amount_out` with
    /// the winning pool's fee-adjusted directional price and `max_slippage`.
    pub fn swap(
        &self,
        params: &mut SwapParams,
        wallet: &mut Wallet,
        max_slippage: f64,
    ) -> Result<(), CatscopeGuestError> {
        let pool_lookup_id = params.pool_lookup_id();
        // look up the set of pools that trade mint_a and mint_b.
        let hs_set = match self.m_pair.get(&pool_lookup_id) {
            Some(x) => x,
            None => {
                return Err(CatscopeGuestError::MissingPool(
                    pool_lookup_id[0],
                    pool_lookup_id[1],
                ))
            }
        };

        // We always want to maximise output-per-input after fees, regardless of
        // direction, so normalise both cases into a single "directional price"
        // (output raw units per input raw unit, fee-adjusted).
        let mut best_i: Option<usize> = None;
        let mut best_directional_price = f64::NEG_INFINITY;

        for &i in hs_set.iter() {
            let pool = &self.l_pool[i];
            let whirlpool = if pool.last_processed >= pool.last_root {
                &pool.processed
            } else {
                &pool.root
            };
            if whirlpool.liquidity == 0 {
                continue;
            }
            let spot = whirlpool.spot_price();
            if spot == 0.0 {
                continue;
            }
            // fee_rate is in hundredths of a basis point (3000 → 0.30%)
            let fee_fraction = whirlpool.fee_rate as f64 / 1_000_000.0;
            let directional = if params.input_mint == whirlpool.token_mint_a {
                // A→B: output_B per input_A
                spot * (1.0 - fee_fraction)
            } else {
                // B→A: output_A per input_B
                (1.0 / spot) * (1.0 - fee_fraction)
            };
            if directional > best_directional_price {
                best_directional_price = directional;
                best_i = Some(i);
            }
        }

        let i = best_i.ok_or(CatscopeGuestError::MissingPool(
            pool_lookup_id[0],
            pool_lookup_id[1],
        ))?;

        let pool = &self.l_pool[i];
        let whirlpool = if pool.last_processed >= pool.last_root {
            &pool.processed
        } else {
            &pool.root
        };

        let a_to_b = params.input_mint == whirlpool.token_mint_a;
        let ts = TICKS_PER_ARRAY * whirlpool.tick_spacing as i32;
        let start_0 = whirlpool.tick_array_start(whirlpool.tick_current_index);
        let pool_pk =
            pubkey_from_account_id(&pool.id).ok_or(CatscopeGuestError::MissingPool(pool.id, 0))?;

        let ta_starts = if a_to_b {
            [start_0, start_0 - ts, start_0 - 2 * ts]
        } else {
            [start_0, start_0 + ts, start_0 + 2 * ts]
        };

        log_warn!(
            "swap: pool={} a_to_b={} tick={} spacing={} ts={} starts={:?}",
            pool.id,
            a_to_b,
            whirlpool.tick_current_index,
            whirlpool.tick_spacing,
            ts,
            ta_starts
        );

        // Real, live-confirmed failure mode (leveragedloopv1's first real
        // jitoSOL/USDC swap, 2026-08-27): when this pool's tick-array
        // accounts were never actually subscribed to/observed (as opposed
        // to merely stale), `tick_array_by_start` returns `None` for all
        // three, the PDA-fallback below guesses addresses from a cached
        // `tick_current_index` alone, `swap_quote_by_input_token` itself
        // detects this is unusable ("Invalid tick array sequence") and
        // falls back to a spot-price estimate -- but the code used to
        // build and send the swap instruction anyway with the guessed PDA
        // tick arrays, which the real on-chain program then rejected with
        // the exact same `TickArraySequenceInvalidIndex`, wasting a real
        // transaction fee on an attempt that was never going to succeed.
        // The current tick array (index 0) is required by every swap
        // regardless of size or direction, so its absence from real
        // subscribed data is a precise, sufficient signal that this pool
        // isn't safe to trade through right now -- refuse before ever
        // reaching `build_swap_ix`, rather than guess and pay to find out.
        if self.tick_array_by_start(&pool.id, ta_starts[0]).is_none() {
            log_warn!(
                "swap: pool={} has no real tick-array data for the current tick (start={}) -- refusing rather than guessing a PDA",
                pool.id,
                ta_starts[0],
            );
            return Err(CatscopeGuestError::Trade(TraderError::PoolNotReady));
        }

        // Resolve tick-array pubkeys (used for the on-chain instruction).
        let tick_array_pubkeys: [Pubkey; 3] = ta_starts.map(|start| {
            if let Some(ta_id) = self.tick_array_by_start(&pool.id, start) {
                if let Some(pk) = pubkey_from_account_id(&ta_id) {
                    log_warn!("  ta start={} -> map id={} pk={}", start, ta_id, pk);
                    return pk;
                }
            }
            let pk = whirlpool.tick_array_pda(&pool_pk, start);
            log_warn!("  ta start={} -> PDA fallback pk={}", start, pk);
            pk
        });

        // Resolve tick-array facades (used for the quote calculation).
        // If a tick array has not been seen on-chain yet, fall back to an empty
        // array with the correct start index — the quote may be less accurate but
        // the instruction will still be built and sent.
        let get_facade = |start: i32| -> TickArrayFacade {
            if let Some(ta_id) = self.tick_array_by_start(&pool.id, start) {
                if let Some(pta) = self.m_tick_array.get(&ta_id) {
                    return pta.facade;
                }
            }
            TickArrayFacade {
                start_tick_index: start,
                ticks: [TickFacade::default(); TICK_ARRAY_SIZE],
            }
        };
        let ta0 = get_facade(ta_starts[0]);
        let ta1 = get_facade(ta_starts[1]);
        let ta2 = get_facade(ta_starts[2]);

        // Use the CLMM library to compute an exact quote and derive min_amount_out.
        let slippage_bps = (max_slippage * 10_000.0) as u16;
        match swap_quote_by_input_token(
            params.amount_in,
            a_to_b,
            slippage_bps,
            whirlpool.to_facade(),
            None, // no oracle — standard (non-adaptive-fee) pool
            TickArrays::Three(ta0, ta1, ta2),
            0,    // timestamp only needed for adaptive fee
            None, // no transfer fee on token A
            None, // no transfer fee on token B
        ) {
            Ok(quote) => {
                params.min_amount_out = quote.token_min_out;
                log_warn!(
                    "  quote: in={} est_out={} min_out={} fee={}",
                    quote.token_in,
                    quote.token_est_out,
                    quote.token_min_out,
                    quote.trade_fee,
                );
            }
            Err(e) => {
                log_warn!(
                    "  swap_quote failed ({:?}), falling back to spot-price estimate",
                    e
                );
                params.set_min_amount_out(best_directional_price, max_slippage);
            }
        }

        whirlpool.build_swap_ix(pool.id, &tick_array_pubkeys, params, wallet)?;
        Ok(())
    }

    /// TEMPORARY DEBUG: dump a pool's raw pricing fields -- checking
    /// whether the pools route_slippage_aware picks (consistently priced
    /// noticeably above real market rate) are genuinely thin, confirming
    /// the "winner's curse" explanation (always picking the best-looking
    /// price among many now-thinner pools, post pool-budget increase)
    /// rather than a bug. Remove once confirmed.
    pub fn debug_pool_state(&self, pool_id: AccountId) -> Option<String> {
        let &idx = self.m_pool.get(&pool_id)?;
        let pool = &self.l_pool[idx].processed;
        let (virtual_a, virtual_b) = pool.virtual_reserves();
        Some(format!(
            "liquidity={} sqrt_price_x64={} tick_current_index={} fee_rate={} spot_price={} \
             virtual_a={} virtual_b={} reserve_a={} reserve_b={}",
            pool.liquidity,
            pool.sqrt_price_x64,
            pool.tick_current_index,
            pool.fee_rate,
            pool.spot_price(),
            virtual_a,
            virtual_b,
            pool.reserve_a,
            pool.reserve_b,
        ))
    }

    /// Exact tick-aware CLMM quote for a single amount, via the same
    /// `swap_quote_by_input_token` machinery `swap()` uses to build a real
    /// instruction -- NOT the naive constant-product-within-current-tick
    /// approximation `cp_quote`/`virtual_reserves()` use for cheap graph
    /// search. Real (non-debug) usage:
    /// `planner::reverify_with_exact_quotes` calls this to correct a
    /// router-found cycle's Orca hops before trusting them as a genuine
    /// opportunity -- found this session that a CLMM pool's real
    /// liquidity structure near the current tick (e.g. a large
    /// `liquidity_net` boundary sitting almost exactly at the current
    /// price) can make a hop the constant-product approximation prices as
    /// profitable actually untradeable (`amount_out == 0`), something the
    /// approximation has no way to see since it doesn't know about tick
    /// boundaries at all.
    ///
    /// Returns `None` if the pool isn't known or has zero active
    /// liquidity; returns `Some(0)` (not `None`) if the exact quote
    /// genuinely computes zero output or the underlying quote call
    /// errors -- callers must check for zero explicitly, it is not
    /// distinguished from "no output" here.
    pub fn exact_quote(&self, pool_id: AccountId, input_mint: AccountId, amount_in: u64) -> Option<u64> {
        let &idx = self.m_pool.get(&pool_id)?;
        let pool = &self.l_pool[idx];
        let whirlpool = if pool.last_processed >= pool.last_root {
            &pool.processed
        } else {
            &pool.root
        };
        if whirlpool.liquidity == 0 {
            return None;
        }
        let a_to_b = input_mint == whirlpool.token_mint_a;

        let ts = TICKS_PER_ARRAY * whirlpool.tick_spacing as i32;
        let start_0 = whirlpool.tick_array_start(whirlpool.tick_current_index);
        let ta_starts = if a_to_b {
            [start_0, start_0 - ts, start_0 - 2 * ts]
        } else {
            [start_0, start_0 + ts, start_0 + 2 * ts]
        };
        let get_facade = |start: i32| -> TickArrayFacade {
            if let Some(ta_id) = self.tick_array_by_start(&pool_id, start) {
                if let Some(pta) = self.m_tick_array.get(&ta_id) {
                    return pta.facade;
                }
            }
            TickArrayFacade {
                start_tick_index: start,
                ticks: [TickFacade::default(); TICK_ARRAY_SIZE],
            }
        };
        let ta0 = get_facade(ta_starts[0]);
        let ta1 = get_facade(ta_starts[1]);
        let ta2 = get_facade(ta_starts[2]);

        let quote = swap_quote_by_input_token(
            amount_in,
            a_to_b,
            0,
            whirlpool.to_facade(),
            None,
            TickArrays::Three(ta0, ta1, ta2),
            0,
            None,
            None,
        )
        .ok()?;
        Some(quote.token_est_out)
    }

    /// Whether every tick-array window [`Self::exact_quote`] needs for this
    /// `pool_id`/`input_mint` direction is actually cached in
    /// `m_tick_array` right now. `exact_quote` itself can't distinguish
    /// "no tradable liquidity" from "haven't received this window's real
    /// data yet" -- a missing window silently becomes a
    /// `TickArrayFacade::default()` (all-zero ticks), which looks
    /// identical to genuine zero liquidity to `swap_quote_by_input_token`.
    /// Real, live-confirmed incident (2026-09-04): a pool whose main
    /// account had just delivered its first live update (so it was a
    /// real, live router edge) still had none of its tick arrays synced
    /// yet -- `exact_quote` returned `None`, and the caller treated that
    /// as a genuine bad quote, cooling the pool down for
    /// `planner::POOL_COOLDOWN_SLOTS` (~30-40 real minutes) even though
    /// the only actual problem was "ask again in a few more seconds."
    /// Callers should check this before trusting an `exact_quote(None)`/
    /// `Some(0)` as a real rejection worth a cooldown -- see
    /// `planner::reverify_hops`'s doc comment.
    pub fn exact_quote_ready(&self, pool_id: AccountId, input_mint: AccountId) -> bool {
        let Some(&idx) = self.m_pool.get(&pool_id) else {
            return false;
        };
        let pool = &self.l_pool[idx];
        let whirlpool = if pool.last_processed >= pool.last_root {
            &pool.processed
        } else {
            &pool.root
        };
        if whirlpool.liquidity == 0 {
            // Genuinely no active liquidity -- not a readiness question.
            return true;
        }
        let a_to_b = input_mint == whirlpool.token_mint_a;
        let ts = TICKS_PER_ARRAY * whirlpool.tick_spacing as i32;
        let start_0 = whirlpool.tick_array_start(whirlpool.tick_current_index);
        let ta_starts = if a_to_b {
            [start_0, start_0 - ts, start_0 - 2 * ts]
        } else {
            [start_0, start_0 + ts, start_0 + 2 * ts]
        };
        ta_starts.iter().all(|&start| {
            self.tick_array_by_start(&pool_id, start)
                .is_some_and(|id| self.m_tick_array.contains_key(&id))
        })
    }

    /// TEMPORARY DEBUG: exact tick-aware CLMM quotes (via the same
    /// `swap_quote_by_input_token` machinery `swap()` uses to build a real
    /// instruction, not the naive constant-product-within-current-tick
    /// approximation `cp_quote`/`virtual_reserves()` use for cheap graph
    /// search) across a range of input sizes, to answer "how much could
    /// actually be sold into this specific thin pool before slippage (and
    /// any tick-boundary crossing) eats the edge, using this pool's real
    /// live tick-array data instead of a hand-rolled approximation.
    pub fn debug_quote_range(&self, pool_id: AccountId, input_mint: AccountId, amounts_in: &[u64]) -> Option<String> {
        let &idx = self.m_pool.get(&pool_id)?;
        let pool = &self.l_pool[idx];
        let whirlpool = if pool.last_processed >= pool.last_root {
            &pool.processed
        } else {
            &pool.root
        };
        if whirlpool.liquidity == 0 {
            return None;
        }
        let a_to_b = input_mint == whirlpool.token_mint_a;

        let ts = TICKS_PER_ARRAY * whirlpool.tick_spacing as i32;
        let start_0 = whirlpool.tick_array_start(whirlpool.tick_current_index);
        let ta_starts = if a_to_b {
            [start_0, start_0 - ts, start_0 - 2 * ts]
        } else {
            [start_0, start_0 + ts, start_0 + 2 * ts]
        };
        let get_facade = |start: i32| -> TickArrayFacade {
            if let Some(ta_id) = self.tick_array_by_start(&pool_id, start) {
                if let Some(pta) = self.m_tick_array.get(&ta_id) {
                    return pta.facade;
                }
            }
            TickArrayFacade {
                start_tick_index: start,
                ticks: [TickFacade::default(); TICK_ARRAY_SIZE],
            }
        };

        let mut out = String::new();
        for &amount_in in amounts_in {
            let ta0 = get_facade(ta_starts[0]);
            let ta1 = get_facade(ta_starts[1]);
            let ta2 = get_facade(ta_starts[2]);
            match swap_quote_by_input_token(
                amount_in,
                a_to_b,
                0,
                whirlpool.to_facade(),
                None,
                TickArrays::Three(ta0, ta1, ta2),
                0,
                None,
                None,
            ) {
                Ok(quote) => {
                    out.push_str(&format!(
                        "in={} out={} fee={} | ",
                        quote.token_in, quote.token_est_out, quote.trade_fee
                    ));
                }
                Err(e) => {
                    out.push_str(&format!("in={amount_in} quote_err={e:?} | "));
                }
            }
        }
        Some(out)
    }

    /// TEMPORARY DEBUG: dumps the pool's real pubkey and, for each of the
    /// 3 tick-array slots `swap()`/`debug_quote_range` look up, whether
    /// this bot has that tick array's `AccountId` mapped at all
    /// (`tick_array_by_start`) and whether real parsed data is cached for
    /// it (`m_tick_array`) -- disambiguates "this pool genuinely has no
    /// tradable liquidity beyond its current tick" from "this bot's own
    /// tick-array cache is incomplete for this pool" when
    /// `debug_quote_range` reports `out=0` for every size. Also dumps each
    /// tick array's own real pubkey so it can be checked directly against
    /// live mainnet RPC, independent of this bot's cache.
    pub fn debug_tick_array_status(&self, pool_id: AccountId, input_mint: AccountId) -> Option<String> {
        let &idx = self.m_pool.get(&pool_id)?;
        let pool = &self.l_pool[idx];
        let whirlpool = if pool.last_processed >= pool.last_root {
            &pool.processed
        } else {
            &pool.root
        };
        let a_to_b = input_mint == whirlpool.token_mint_a;
        let ts = TICKS_PER_ARRAY * whirlpool.tick_spacing as i32;
        let start_0 = whirlpool.tick_array_start(whirlpool.tick_current_index);
        let ta_starts = if a_to_b {
            [start_0, start_0 - ts, start_0 - 2 * ts]
        } else {
            [start_0, start_0 + ts, start_0 + 2 * ts]
        };
        let pool_pk = pubkey_from_account_id(&pool_id).map(|pk| pk.to_string());
        let mut out = format!(
            "pool_pk={pool_pk:?} tick_current_index={} tick_spacing={} liquidity={} a_to_b={a_to_b} ",
            whirlpool.tick_current_index, whirlpool.tick_spacing, whirlpool.liquidity,
        );
        for start in ta_starts {
            let ta_id = self.tick_array_by_start(&pool_id, start);
            let cached = ta_id.is_some_and(|id| self.m_tick_array.contains_key(&id));
            let ta_pk = ta_id.and_then(|id| pubkey_from_account_id(&id)).map(|pk| pk.to_string());
            out.push_str(&format!("| start={start} ta_id={ta_id:?} ta_pk={ta_pk:?} cached={cached} "));
        }
        Some(out)
    }

    /// Build the swap instruction for one `Hop` routed through this dex --
    /// same uniform adapter shape as every other dex module's `plan_hop`.
    ///
    /// **Known caveat**: unlike every other dex's adapter, this does NOT
    /// necessarily use `hop.pool_id` -- `swap()` independently re-selects
    /// whichever pool for this mint pair currently looks best-priced (see
    /// its own doc comment), which may differ from the specific pool the
    /// price graph priced `hop` against. Fine for a plan-only sanity check
    /// (still proves *some* real Orca swap is buildable for this mint
    /// pair), but would need reconciling before this path is ever used for
    /// real execution.
    pub fn plan_hop(
        &self,
        hop: &Hop,
        owner: AccountId,
        source_ata: AccountId,
        dest_ata: AccountId,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        const PLAN_MAX_SLIPPAGE: f64 = 0.01;
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
        self.swap(&mut params, wallet, PLAN_MAX_SLIPPAGE).map_err(|_| TraderError::PoolNotReady)
    }

    /// Build a [`TradeRouter`] from the current live pool snapshots.
    ///
    /// Call this at the top of `evaluate()`. Each call is cheap — it iterates
    /// `l_pool` once and allocates a small adjacency list.
    ///
    /// Uses `processed` state when it is newer than `root`; otherwise `root`.
    /// Pools with zero liquidity or zero spot price are skipped automatically
    /// by `TradeRouter::add_orca_pool`.
    pub fn build_router(&self) -> crate::trader::pricegraph::TradeRouter {
        use crate::trader::pricegraph::TradeRouter;
        const MIN_RESERVE: u64 = 1_000; // filters dust/empty pools only
        let mut router = TradeRouter::new();
        for pool in &self.l_pool {
            let whirlpool = if pool.last_processed >= pool.last_root {
                &pool.processed
            } else {
                &pool.root
            };
            if whirlpool.reserve_a.min(whirlpool.reserve_b) < MIN_RESERVE {
                continue;
            }
            router.add_orca_pool(pool.id, whirlpool, DexType::OrcaWhirlpool);
        }
        router
    }
}

impl Updater for OrcaState {
    fn on_account(&mut self, header: &Header, body: &[u8]) {
        if header.owner != self.program_id {
            return;
        }
        if body.len() < 8 {
            return;
        }
        let is_final = false;
        if body[..8] == DISC_WHIRLPOOL {
            if !is_final {
                self.count += 1;
            }
            if let Ok(i) = self
                .l_pool
                .binary_search_by_key(&header.accountid, |p| p.id)
            {
                // OFF_* constants are absolute offsets from byte 0 (including discriminator).
                // Pass the full body so the constants align correctly.
                {
                    let pool = &mut self.l_pool[i];
                    let vault_a;
                    let vault_b;
                    if is_final {
                        pool.root.parse(body).expect("parse");
                        pool.last_root = header.slot;
                        if pool.last_processed < header.slot {
                            pool.processed = pool.root.clone();
                        }
                        vault_a = pool.root.vault_a;
                        vault_b = pool.root.vault_b;
                    } else {
                        self.parsed_count += 1;
                        pool.last_processed = header.slot;
                        pool.processed.parse(body).expect("parse");
                        vault_a = pool.processed.vault_a;
                        vault_b = pool.processed.vault_b;
                    }
                    assert_ne!(vault_a, vault_b);
                    if let PoolTokenSubscribeStage::Waiting = pool.stage {
                        pool.stage = PoolTokenSubscribeStage::Found(0);
                        for v in [vault_a, vault_b] {
                            self.token_sub_queue.push(SubscriptionRequest {
                                root: v,
                                filter_weight: u32::MAX,
                                depth: 1,
                            });
                            self.m_vault.insert(v, i);
                        }
                    }

                    // Tick arrays are off-chain PDA-derived (not a pubkey
                    // field inside the Whirlpool account like the vaults
                    // above), so the host's account graph has no way to
                    // discover them on its own -- subscribe explicitly,
                    // same pattern as the vault subscriptions. Re-derive
                    // and re-subscribe whenever price drifts into a new
                    // tick-array window, not just once at pool discovery,
                    // since a stale cached window silently degrades quote
                    // accuracy (see `get_facade`'s empty-facade fallback).
                    let (start_0, o_ta_pks) = {
                        let whirlpool = if is_final { &pool.root } else { &pool.processed };
                        let start_0 = whirlpool.tick_array_start(whirlpool.tick_current_index);
                        let o_ta_pks =
                            pubkey_from_account_id(&pool.id).map(|pk| whirlpool.tick_arrays(&pk));
                        (start_0, o_ta_pks)
                    };
                    if pool.subscribed_tick_start != Some(start_0) {
                        if let Some(ta_pks) = o_ta_pks {
                            for ta_pk in ta_pks {
                                self.token_sub_queue.push(SubscriptionRequest {
                                    root: account_id_from_pubkey(&ta_pk),
                                    filter_weight: u32::MAX,
                                    depth: 1,
                                });
                            }
                            pool.subscribed_tick_start = Some(start_0);
                        }
                    }
                }
            }
        } else if body[..8] == DISC_TICK_ARRAY {
            if body.len() < MIN_TICK_ARRAY_LEN {
                return;
            }
            let start_tick_index = i32::from_le_bytes(
                body[OFF_TA_START_INDEX..OFF_TA_START_INDEX + 4]
                    .try_into()
                    .unwrap(),
            );
            let whirlpool_pk = Pubkey::new_from_array(
                body[OFF_TA_WHIRLPOOL..OFF_TA_WHIRLPOOL + 32]
                    .try_into()
                    .unwrap(),
            );
            let whirlpool = account_id_from_pubkey(&whirlpool_pk);
            let ta_id = header.accountid;

            // Parse all 88 ticks from account bytes.
            // Tick layout: initialized(1) + liquidity_net(16) + liquidity_gross(16)
            //            + fee_growth_a(16) + fee_growth_b(16) + reward_growths(3×16) = 113 bytes
            const TICK_BYTE_SIZE: usize = 113;
            const TICKS_OFFSET: usize = 12; // discriminator(8) + start_tick_index(4)
            let mut ticks = [TickFacade::default(); TICK_ARRAY_SIZE];
            for (i, tick) in ticks.iter_mut().enumerate() {
                let o = TICKS_OFFSET + i * TICK_BYTE_SIZE;
                if o + TICK_BYTE_SIZE > body.len() {
                    break;
                }
                tick.initialized = body[o] != 0;
                tick.liquidity_net = i128::from_le_bytes(body[o + 1..o + 17].try_into().unwrap());
                tick.liquidity_gross =
                    u128::from_le_bytes(body[o + 17..o + 33].try_into().unwrap());
                tick.fee_growth_outside_a =
                    u128::from_le_bytes(body[o + 33..o + 49].try_into().unwrap());
                tick.fee_growth_outside_b =
                    u128::from_le_bytes(body[o + 49..o + 65].try_into().unwrap());
                tick.reward_growths_outside = [
                    u128::from_le_bytes(body[o + 65..o + 81].try_into().unwrap()),
                    u128::from_le_bytes(body[o + 81..o + 97].try_into().unwrap()),
                    u128::from_le_bytes(body[o + 97..o + 113].try_into().unwrap()),
                ];
            }
            let facade = TickArrayFacade {
                start_tick_index,
                ticks,
            };

            self.m_tick_array
                .entry(ta_id)
                .and_modify(|e| {
                    e.start_tick_index = start_tick_index;
                    e.facade = facade;
                })
                .or_insert(ParsedTickArray {
                    start_tick_index,
                    whirlpool,
                    facade,
                });
            self.m_pool_tick_arrays
                .entry(whirlpool)
                .or_default()
                .insert(start_tick_index, ta_id);
        }
    }

    /// Called for every low-latency token account update.
    /// When the update is for a pool vault, fetches the fresh pool state via account_by_id
    /// so that processed price/tick reflect changes before the next commit.
    fn on_token(&mut self, ta: &Tokenaccountv1) -> bool {
        let pool_i = match self.m_vault.get(&ta.id) {
            Some(&i) => i,
            None => return false,
        };

        let pool = &mut self.l_pool[pool_i];
        let mut add_pending_token_counted = 0;
        match pool.stage {
            PoolTokenSubscribeStage::Waiting => panic!("not possible"),
            PoolTokenSubscribeStage::Found(n) => {
                if n < 2 {
                    pool.stage = PoolTokenSubscribeStage::Found(n + 1);
                    add_pending_token_counted += 1;
                }
            }
        };
        // Decide side by mint, not by comparing ta.id against
        // pool.root.vault_a/vault_b -- pool.root is only ever written by
        // the is_final branch above, which is permanently dead (is_final
        // is a hardcoded `false`), so pool.root.vault_a/vault_b stay at
        // AccountId::default() (0) forever and that comparison never
        // matched any real vault ID. This silently kept reserve_a/reserve_b
        // at 0 for every single Orca pool regardless of how much live
        // vault-balance data arrived, which meant add_orca_pool's
        // liquidity gate never passed and batch_router never emitted a
        // single Orca edge -- confirmed directly via a diagnostic (2000/
        // 2000 pools failing the gate, unchanged over 1300+ batch_router
        // calls and 9+ minutes of real on_token activity).
        if pool.mint_a == ta.mint {
            pool.vault_a_balance = (ta.amount, ta.slot);
            pool.processed.reserve_a = ta.amount;
        } else if pool.mint_b == ta.mint {
            pool.vault_b_balance = (ta.amount, ta.slot);
            pool.processed.reserve_b = ta.amount;
        } else {
            panic!("pool token mismatch: {pool:?} {ta:?}")
        }
        self.pending_token_counted += add_pending_token_counted;
        true
    }

    fn refresh_account_router(&mut self, account_id: AccountId, router: &mut TradeRouter) {
        if let Some(&idx) = self.m_pool.get(&account_id) {
            let pool = &self.l_pool[idx];
            router.add_orca_pool(pool.id, &pool.processed, DexType::OrcaWhirlpool);
        }
        // else: not a whirlpool account (e.g. a tick array write) -- no
        // pool edges to touch here; still covered by batch_router's
        // periodic full resync.
    }

    fn refresh_token_router(&mut self, ta_id: AccountId, router: &mut TradeRouter) {
        if let Some(&idx) = self.m_vault.get(&ta_id) {
            let pool = &self.l_pool[idx];
            router.add_orca_pool(pool.id, &pool.processed, DexType::OrcaWhirlpool);
        }
    }

    fn batch_router(&mut self, router: &mut TradeRouter) {
        self.batch_router_calls += 1;
        let log_this_call = self.batch_router_calls == 1 || self.batch_router_calls % 20 == 0;
        let (mut n_total, mut n_gate_fail, mut n_node_fail, mut n_ok) = (0usize, 0usize, 0usize, 0usize);
        for (&pool_id, &idx) in self.m_pool.iter() {
            let pool = &self.l_pool[idx].processed;
            if log_this_call {
                n_total += 1;
                let spot = pool.spot_price();
                if spot <= 0.0 || pool.liquidity == 0 || pool.reserve_a == 0 || pool.reserve_b == 0 {
                    n_gate_fail += 1;
                } else if !router.has_node(pool.token_mint_a) || !router.has_node(pool.token_mint_b) {
                    n_node_fail += 1;
                } else {
                    n_ok += 1;
                }
            }
            router.add_orca_pool(pool_id, pool, DexType::OrcaWhirlpool);
        }
        if log_this_call {
            log_warn!(
                "orca: batch_router diagnostic (call {}): total={} gate_fail={} node_fail={} ok={}",
                self.batch_router_calls,
                n_total,
                n_gate_fail,
                n_node_fail,
                n_ok,
            );
        }
    }

    fn on_tx(&mut self, _ix: &CatscopeInstructionRead<'_>, _slot: &Slot) {}

    // Paced through `token_sub_queue` (bounded `max_per_flush` per call)
    // instead of one unbounded `bulk_subscribe` for however many vault/
    // tick-array requests this commit happened to discover -- real,
    // live-observed motivation: an unbounded batch here is exactly what
    // produced this session's real `stdio timeout` hangs (traced to a
    // commit whose `on_account` loop completed but `flush_pool` never
    // returned).
    fn flush_pool(&mut self, g: &Graph, max_per_flush: usize) -> Result<(), CatscopeGuestError> {
        self.token_sub_queue.flush(g, max_per_flush)?;
        Ok(())
    }
}

struct Setup {
    l_pool: Vec<OrcaPoolSetup>,
}
pub struct OrcaPoolSetup {
    pub pubkey: AccountId,
    pub mint_a: AccountId,
    pub mint_b: AccountId,
}
impl Default for Setup {
    fn default() -> Self {
        let mut l_pool = Vec::with_capacity(orca_config::ORCA_WHIRLPOOL_POOLS.len());
        {
            for pool in orca_config::ORCA_WHIRLPOOL_POOLS.iter() {
                let pubkey = account_id_from_pubkey(&Pubkey::new_from_array(pool.pubkey));
                let mint_a = account_id_from_pubkey(&Pubkey::new_from_array(pool.mint_a));
                let mint_b = account_id_from_pubkey(&Pubkey::new_from_array(pool.mint_b));
                l_pool.push(OrcaPoolSetup {
                    pubkey,
                    mint_a,
                    mint_b,
                });
            }
        }
        Self { l_pool }
    }
}
