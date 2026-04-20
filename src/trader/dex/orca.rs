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
    graph::AccountId,
    trader::types::{PoolPrice, SwapParams, TraderError},
    util::{account_id_from_pubkey, pubkey_from_account_id},
    wallet::Wallet,
};
use solana_sdk::{
    message::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

// ─── Program IDs ─────────────────────────────────────────────────────────────

pub const ORCA_WHIRLPOOL_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc");

pub const SPL_TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

// ─── Whirlpool account offsets ────────────────────────────────────────────────

const OFF_TICK_SPACING: usize = 41;
const OFF_FEE_RATE: usize = 45;
const OFF_SQRT_PRICE: usize = 65;
const OFF_TICK_CURRENT: usize = 81;
const OFF_MINT_A: usize = 101;
const OFF_VAULT_A: usize = 133;
const OFF_MINT_B: usize = 181;
const OFF_VAULT_B: usize = 213;

const MIN_WHIRLPOOL_LEN: usize = OFF_VAULT_B + 32;

/// Number of initialized ticks per tick-array account.
const TICK_ARRAY_SIZE: i32 = 88;

// ─── Parsed pool state ────────────────────────────────────────────────────────

/// Parsed state of an Orca Whirlpool pool.
#[derive(Debug, Clone)]
pub struct OrcaWhirlpool {
    /// Token A mint.
    pub token_mint_a: AccountId,
    /// Token B mint.
    pub token_mint_b: AccountId,
    /// Pool's token A vault (subscribe for reserve updates).
    pub vault_a: AccountId,
    /// Pool's token B vault (subscribe for reserve updates).
    pub vault_b: AccountId,
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
        let size = TICK_ARRAY_SIZE * self.tick_spacing as i32;
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
        Pubkey::find_program_address(
            &[b"oracle", pool_pk.as_ref()],
            &ORCA_WHIRLPOOL_PROGRAM_ID,
        )
        .0
    }

    /// Compute the three tick-array PDAs needed for a swap.
    ///
    /// `a_to_b = true` means price decreases (tick goes down).
    pub fn tick_arrays(&self, pool_pk: &Pubkey, a_to_b: bool) -> [Pubkey; 3] {
        let ts = TICK_ARRAY_SIZE * self.tick_spacing as i32;
        let start_0 = self.tick_array_start(self.tick_current_index);
        let (start_1, start_2) = if a_to_b {
            (start_0 - ts, start_0 - 2 * ts)
        } else {
            (start_0 + ts, start_0 + 2 * ts)
        };
        [
            self.tick_array_pda(pool_pk, start_0),
            self.tick_array_pda(pool_pk, start_1),
            self.tick_array_pda(pool_pk, start_2),
        ]
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
    let read_pk = |off: usize| -> Pubkey {
        Pubkey::new_from_array(body[off..off + 32].try_into().unwrap())
    };
    let pk_id = |pk: Pubkey| account_id_from_pubkey(&pk);

    Some(OrcaWhirlpool {
        token_mint_a: pk_id(read_pk(OFF_MINT_A)),
        token_mint_b: pk_id(read_pk(OFF_MINT_B)),
        vault_a: pk_id(read_pk(OFF_VAULT_A)),
        vault_b: pk_id(read_pk(OFF_VAULT_B)),
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

/// Anchor discriminator for the `swap` instruction.
///
/// First 8 bytes of `sha256("global:swap")`.
pub const SWAP_DISCRIMINATOR: [u8; 8] = [248, 198, 158, 145, 225, 117, 135, 200];

/// Append an Orca Whirlpool `swap` instruction to the wallet queue.
///
/// Tick-array PDAs are derived automatically from the pool's current tick
/// and tick spacing. `sqrt_price_limit` is set to 0 (no limit).
pub fn build_swap_ix(
    pool_id: AccountId,
    pool: &OrcaWhirlpool,
    params: &SwapParams,
    wallet: &mut Wallet,
) -> Result<(), TraderError> {
    let a_to_b = if params.input_mint == pool.token_mint_a
        && params.output_mint == pool.token_mint_b
    {
        true
    } else if params.input_mint == pool.token_mint_b && params.output_mint == pool.token_mint_a {
        false
    } else {
        return Err(TraderError::WrongMints);
    };

    let resolve = |id: AccountId| -> Result<Pubkey, TraderError> {
        pubkey_from_account_id(&id).ok_or(TraderError::PubkeyResolutionFailed(id))
    };

    let pool_pk = resolve(pool_id)?;
    let vault_a_pk = resolve(pool.vault_a)?;
    let vault_b_pk = resolve(pool.vault_b)?;
    let user_source_pk = resolve(params.user_source_token_account)?;
    let user_dest_pk = resolve(params.user_destination_token_account)?;
    let user_wallet_pk = resolve(params.user_wallet)?;

    let tick_arrays = pool.tick_arrays(&pool_pk, a_to_b);
    let tick_array_pks = tick_arrays.map(|pk| {
        account_id_from_pubkey(&pk);
        pk
    });

    let oracle_pk = pool.oracle_pda(&pool_pk);

    // Instruction data (43 bytes)
    let mut data = Vec::with_capacity(43);
    data.extend_from_slice(&SWAP_DISCRIMINATOR);
    data.extend_from_slice(&params.amount_in.to_le_bytes());           // amount
    data.extend_from_slice(&params.min_amount_out.to_le_bytes());      // other_amount_threshold
    data.extend_from_slice(&0u128.to_le_bytes());                      // sqrt_price_limit (no limit)
    data.push(1u8);                                                     // amount_specified_is_input = true
    data.push(if a_to_b { 1u8 } else { 0u8 });                        // a_to_b

    // Determine which user token accounts are A and B
    let (user_a_pk, user_b_pk) = if a_to_b {
        (user_source_pk, user_dest_pk)
    } else {
        (user_dest_pk, user_source_pk)
    };

    let accounts = vec![
        AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),
        AccountMeta::new_readonly(user_wallet_pk, true),  // token_authority (signer)
        AccountMeta::new(pool_pk, false),
        AccountMeta::new(user_a_pk, false),
        AccountMeta::new(vault_a_pk, false),
        AccountMeta::new(user_b_pk, false),
        AccountMeta::new(vault_b_pk, false),
        AccountMeta::new(tick_array_pks[0], false),
        AccountMeta::new(tick_array_pks[1], false),
        AccountMeta::new(tick_array_pks[2], false),
        AccountMeta::new_readonly(oracle_pk, false),
    ];

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
