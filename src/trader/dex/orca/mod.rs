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
    catscope::witbot::shooter::{self, Header, Tokenaccountv1},
    err::CatscopeGuestError,
    graph::{AccountId, Graph, Subscription, SubscriptionRequest},
    log_debug, log_warn,
    token::{PairAmount, TradeAmount},
    trader::types::{PoolPrice, SwapParams, TraderError},
    txview::{CatscopeInstruction, CatscopeInstructionRead, CatscopeTransactionReadWrapper},
    util::{account_id_from_pubkey, pubkey_from_account_id},
    wallet::Wallet,
    TradingSetup,
};
use orca_whirlpools_core::{
    swap_quote_by_input_token, TickArrayFacade, TickArrays, TickFacade, WhirlpoolFacade,
    TICK_ARRAY_SIZE,
};
use solana_sdk::{
    clock::Slot,
    message::{AccountMeta, Instruction},
    pubkey::Pubkey,
    transaction::TransactionError,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    hash::BuildHasherDefault,
    u32,
};

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
    // map [mint_a,mint_b]->pool AccountId
    m_pair: HashMap<[AccountId; 2], HashSet<usize>>,
    q_update_pool: VecDeque<AccountId>,
    // map vault token account id -> pool index in l_pool
    m_vault: HashMap<AccountId, usize>,
    // map tick_array account_id -> parsed tick array
    m_tick_array: HashMap<AccountId, ParsedTickArray>,
    // map pool_id -> { start_tick_index -> tick_array account_id }
    m_pool_tick_arrays: HashMap<AccountId, HashMap<i32, AccountId>>,
    l_token_sub: Vec<SubscriptionRequest>,
    q_hold_sub: VecDeque<Vec<Subscription>>,
    pending_token_counted: usize,
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
    o_sub: Option<Subscription>,
    last_root: Slot,
    root: OrcaWhirlpool,
    last_processed: Slot,
    processed: OrcaWhirlpool,
    last_tx: Slot,
    mint_a: AccountId,
    vault_a_balance: (u64, Slot),
    mint_b: AccountId,
    vault_b_balance: (u64, Slot),
}

impl Drop for PoolInfo {
    fn drop(&mut self) {
        let _ignore = self.o_sub.take();
    }
}
impl OrcaState {
    pub fn new(g: &Graph, trading: &TradingSetup) -> Result<Self, CatscopeGuestError> {
        let l_ops = trading.orca();
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
            // subscribe to each pool and corresponding tick arrays
            l_req.push(SubscriptionRequest {
                root: ops.pubkey,
                filter_weight: u32::MAX,
                depth: 2,
            });
        }
        let req_n = l_req.len();
        log_warn!("OrcaState::subscribe {req_n} - 1");
        let l_sub = g.bulk_subscribe(l_req)?;
        log_warn!("OrcaState::subscribe {req_n} - 2");
        assert_eq!(l_sub.len(), req_n);
        for (i, s) in l_sub.into_iter().enumerate() {
            let ops = &l_ops[i];
            l_pool.push(PoolInfo {
                vault_a_balance: (0, 0),
                vault_b_balance: (0, 0),
                stage: PoolTokenSubscribeStage::Waiting,
                last_tx: 0,
                id: ops.pubkey,
                o_sub: Some(s),
                last_root: 0,
                root: OrcaWhirlpool::default(),
                last_processed: 0,
                processed: OrcaWhirlpool::default(),
                mint_a: ops.mint_a,
                mint_b: ops.mint_b,
            });
        }
        let mut m_pair = HashMap::with_capacity(2 * l_ops.len());
        l_pool.sort_unstable_by_key(|p| p.id);
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

        Ok(Self {
            pending_token_counted: 0,
            has_check_pool,
            count: 0,
            parsed_count: 0,
            tx_count: 0,
            l_pool,
            m_pair,
            q_update_pool: VecDeque::with_capacity(20),
            program_id: orca_program_id,
            m_vault: HashMap::with_capacity(100_000),
            m_tick_array: HashMap::new(),
            m_pool_tick_arrays: HashMap::new(),
            l_token_sub: Vec::with_capacity(100_000),
            q_hold_sub: VecDeque::new(),
        })
    }

    #[inline]
    pub fn program_id(&self) -> &AccountId {
        &self.program_id
    }

    /// Look up all known tick arrays for a pool, keyed by start_tick_index.
    pub fn tick_arrays_for_pool(&self, pool_id: &AccountId) -> Option<&HashMap<i32, AccountId>> {
        self.m_pool_tick_arrays.get(pool_id)
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

    pub fn flush_pool(&mut self, g: &Graph) -> Result<(), CatscopeGuestError> {
        let mut l_req = Vec::with_capacity(1_000);
        std::mem::swap(&mut l_req, &mut self.l_token_sub);
        let l_sub_id = g.bulk_subscribe(l_req)?;
        self.q_hold_sub.push_back(l_sub_id);
        Ok(())
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
    }

    pub fn on_account(
        &mut self,
        header: &Header,
        body: &[u8],
        is_final: bool,
    ) -> Result<(), CatscopeGuestError> {
        if header.owner != self.program_id {
            return Ok(());
        }
        if body.len() < 8 {
            return Ok(());
        }
        if body[..8] == DISC_WHIRLPOOL {
            if !is_final {
                self.count += 1;
            }
            if let Some(i) = self
                .l_pool
                .binary_search_by_key(&header.accountid, |p| p.id)
                .ok()
            {
                // OFF_* constants are absolute offsets from byte 0 (including discriminator).
                // Pass the full body so the constants align correctly.
                {
                    let pool = &mut self.l_pool[i];
                    let vault_a;
                    let vault_b;
                    if is_final {
                        pool.root.parse(body)?;
                        pool.last_root = header.slot;
                        if pool.last_processed < header.slot {
                            pool.processed = pool.root.clone();
                        }
                        vault_a = pool.root.vault_a;
                        vault_b = pool.root.vault_b;
                    } else {
                        self.parsed_count += 1;
                        pool.last_processed = header.slot;
                        pool.processed.parse(body)?;
                        vault_a = pool.processed.vault_a;
                        vault_b = pool.processed.vault_b;
                    }
                    assert_ne!(vault_a, vault_b);
                    if let PoolTokenSubscribeStage::Waiting = pool.stage {
                        pool.stage = PoolTokenSubscribeStage::Found(0);
                        for v in [vault_a, vault_b] {
                            self.l_token_sub.push(SubscriptionRequest {
                                root: v,
                                filter_weight: u32::MAX,
                                depth: 1,
                            });
                            self.m_vault.insert(v, i);
                        }
                    }
                }
            }
        } else if body[..8] == DISC_TICK_ARRAY {
            if body.len() < MIN_TICK_ARRAY_LEN {
                return Ok(());
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
        Ok(())
    }

    /// Called for every low-latency token account update.
    /// When the update is for a pool vault, fetches the fresh pool state via account_by_id
    /// so that processed price/tick reflect changes before the next commit.
    pub fn on_token(&mut self, ta: &Tokenaccountv1) -> Result<(), CatscopeGuestError> {
        let pool_i = match self.m_vault.get(&ta.id) {
            Some(&i) => i,
            None => return Ok(()),
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
        if pool.mint_a == ta.mint {
            pool.vault_a_balance = (ta.amount, ta.slot);
        } else if pool.mint_b == ta.mint {
            pool.vault_b_balance = (ta.amount, ta.slot);
        } else {
            panic!("pool token mismatch: {pool:?} {ta:?}")
        }
        // Update the reserve we know from the token account directly.
        {
            let pool2 = &mut self.l_pool[pool_i];
            if ta.id == pool2.root.vault_a {
                pool2.processed.reserve_a = ta.amount;
            } else if ta.id == pool2.root.vault_b {
                pool2.processed.reserve_b = ta.amount;
            }
        }
        self.pending_token_counted += add_pending_token_counted;
        Ok(())
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
}

pub struct OrcaPoolSetup {
    pub pubkey: AccountId,
    pub mint_a: AccountId,
    pub mint_b: AccountId,
}
