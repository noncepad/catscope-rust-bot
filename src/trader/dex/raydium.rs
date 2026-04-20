//! Raydium AMM v4 (constant-product, OpenBook-backed) parser and swap builder.
//!
//! # Account layout — Raydium AMM v4 pool state
//!
//! Raydium AMM v4 is a *native* Solana program (no Anchor discriminator).
//! All integer fields are little-endian. Key offsets:
//!
//! ```text
//! offset   size  field
//! ──────   ────  ─────────────────────────────────────────
//!    0       8   status (u64)
//!    8       8   nonce (u64)            ← used to derive amm_authority PDA
//!   16       8   maxOrder (u64)
//!   24       8   depth (u64)
//!   32       8   baseDecimal (u64)
//!   40       8   quoteDecimal (u64)
//!   48 –175  …   (trading/fee params, 16 fields × 8 bytes = 128 bytes)
//!  176       8   swapFeeNumerator (u64)
//!  184       8   swapFeeDenominator (u64)
//!  192 –327  …   (pnl / swap volume accumulators)
//!  328       8   swapPc2CoinFee (u64)
//!  336      32   baseVault   (Pubkey) ← coin vault token account
//!  368      32   quoteVault  (Pubkey) ← pc vault token account
//!  400      32   baseMint    (Pubkey)
//!  432      32   quoteMint   (Pubkey)
//!  464      32   lpMint      (Pubkey)
//!  496      32   openOrders  (Pubkey)
//!  528      32   marketId    (Pubkey) ← OpenBook market
//!  560      32   marketProgramId (Pubkey)
//!  592      32   targetOrders (Pubkey)
//! ```

use crate::{
    graph::AccountId,
    trader::types::{PoolPrice, SwapParams, TraderError},
    util::pubkey_from_account_id,
    wallet::Wallet,
};
use solana_sdk::{
    message::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

// ─── Program IDs ─────────────────────────────────────────────────────────────

pub const RAYDIUM_AMM_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8");

/// Single authority PDA shared by all Raydium AMM v4 pools.
/// Derived from `find_program_address(&[b"amm authority"], &RAYDIUM_AMM_PROGRAM_ID)`.
pub const RAYDIUM_AMM_AUTHORITY: Pubkey =
    Pubkey::from_str_const("5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1");

/// SPL Token program.
pub const SPL_TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

// ─── Pool state offsets ───────────────────────────────────────────────────────

const OFF_SWAP_FEE_NUM: usize = 176;
const OFF_SWAP_FEE_DEN: usize = 184;
const OFF_BASE_VAULT: usize = 336;
const OFF_QUOTE_VAULT: usize = 368;
const OFF_BASE_MINT: usize = 400;
const OFF_QUOTE_MINT: usize = 432;
const OFF_OPEN_ORDERS: usize = 496;
const OFF_MARKET_ID: usize = 528;
const OFF_MARKET_PROGRAM: usize = 560;
const OFF_TARGET_ORDERS: usize = 592;

/// Minimum byte length for a valid Raydium AMM v4 pool state account.
const MIN_POOL_STATE_LEN: usize = OFF_TARGET_ORDERS + 32;

// ─── Parsed pool state ────────────────────────────────────────────────────────

/// Parsed state of a Raydium AMM v4 pool.
///
/// Extracted from the on-chain pool state account via [`parse`].
/// Reserve fields (`reserve_a` / `reserve_b`) are populated separately
/// through [`on_token`](crate::trader::DexTrader::on_token) when the vault
/// SPL token accounts are updated.
#[derive(Debug, Clone)]
pub struct RaydiumAmmPool {
    /// Coin (base) mint.
    pub coin_mint: AccountId,
    /// PC (quote) mint.
    pub pc_mint: AccountId,
    /// Coin vault SPL token account — subscribe to this for reserve updates.
    pub coin_vault: AccountId,
    /// PC vault SPL token account — subscribe to this for reserve updates.
    pub pc_vault: AccountId,
    /// AMM open-orders account (Serum/OpenBook order book).
    pub open_orders: AccountId,
    /// AMM target-orders account.
    pub target_orders: AccountId,
    /// OpenBook/Serum market account.
    pub market_id: AccountId,
    /// OpenBook/Serum program (e.g. `srmqPvymJeFKQ4zGQed1GFppgkRHL9kaELCbyksJtPX`).
    pub market_program: AccountId,
    /// Swap fee numerator.
    pub swap_fee_numerator: u64,
    /// Swap fee denominator.
    pub swap_fee_denominator: u64,
    /// Cached coin reserve (raw lamports). Updated by vault token account events.
    pub reserve_coin: u64,
    /// Cached PC reserve (raw lamports). Updated by vault token account events.
    pub reserve_pc: u64,
}

impl RaydiumAmmPool {
    /// Fee in basis points, derived from the pool's swap fee fields.
    pub fn fee_bps(&self) -> u16 {
        if self.swap_fee_denominator == 0 {
            return 0;
        }
        ((self.swap_fee_numerator * 10_000 / self.swap_fee_denominator) as u16).min(10_000)
    }

    /// Build a [`PoolPrice`] snapshot. Returns `None` if either reserve is 0.
    pub fn pool_price(&self) -> Option<PoolPrice> {
        if self.reserve_coin == 0 || self.reserve_pc == 0 {
            return None;
        }
        let price = self.reserve_pc as f64 / self.reserve_coin as f64;
        Some(PoolPrice {
            token_a: self.coin_mint,
            token_b: self.pc_mint,
            price,
            reserve_a: self.reserve_coin,
            reserve_b: self.reserve_pc,
            fee_bps: self.fee_bps(),
        })
    }
}

/// Additional OpenBook/Serum market accounts required to build a swap instruction.
///
/// These cannot be derived from the pool state alone; obtain them from the
/// OpenBook market account or Raydium's pool-info API, then configure once.
#[derive(Debug, Clone)]
pub struct RaydiumAmmSwapConfig {
    /// OpenBook bids account (writable).
    pub market_bids: AccountId,
    /// OpenBook asks account (writable).
    pub market_asks: AccountId,
    /// OpenBook event queue account (writable).
    pub market_event_queue: AccountId,
    /// OpenBook coin (base) vault (writable).
    pub market_coin_vault: AccountId,
    /// OpenBook pc (quote) vault (writable).
    pub market_pc_vault: AccountId,
    /// OpenBook vault signer PDA (read-only).
    pub market_vault_signer: AccountId,
}

// ─── Parser ───────────────────────────────────────────────────────────────────

/// Parse a Raydium AMM v4 pool state from raw account body bytes.
///
/// Returns `None` if `body` is too short to be a valid pool state.
pub fn parse(body: &[u8]) -> Option<RaydiumAmmPool> {
    if body.len() < MIN_POOL_STATE_LEN {
        return None;
    }

    let read_u64 = |off: usize| u64::from_le_bytes(body[off..off + 8].try_into().unwrap());
    let read_pk = |off: usize| -> Pubkey {
        Pubkey::new_from_array(body[off..off + 32].try_into().unwrap())
    };
    let pk_id = |pk: Pubkey| crate::util::account_id_from_pubkey(&pk);

    Some(RaydiumAmmPool {
        coin_mint: pk_id(read_pk(OFF_BASE_MINT)),
        pc_mint: pk_id(read_pk(OFF_QUOTE_MINT)),
        coin_vault: pk_id(read_pk(OFF_BASE_VAULT)),
        pc_vault: pk_id(read_pk(OFF_QUOTE_VAULT)),
        open_orders: pk_id(read_pk(OFF_OPEN_ORDERS)),
        target_orders: pk_id(read_pk(OFF_TARGET_ORDERS)),
        market_id: pk_id(read_pk(OFF_MARKET_ID)),
        market_program: pk_id(read_pk(OFF_MARKET_PROGRAM)),
        swap_fee_numerator: read_u64(OFF_SWAP_FEE_NUM),
        swap_fee_denominator: read_u64(OFF_SWAP_FEE_DEN),
        reserve_coin: 0,
        reserve_pc: 0,
    })
}

// ─── Swap instruction builder ─────────────────────────────────────────────────

/// Compute units budgeted for a Raydium AMM v4 swap.
pub const RAYDIUM_AMM_SWAP_CU: u32 = 200_000;

/// Append a Raydium AMM v4 `SwapBaseIn` instruction to the wallet queue.
///
/// `SwapBaseIn` spends exactly `params.amount_in` of the input token and
/// receives at least `params.min_amount_out` of the output token.
///
/// # Instruction data (17 bytes)
/// ```text
/// [0]     discriminant = 9 (SwapBaseIn)
/// [1..9]  amount_in          (u64 LE)
/// [9..17] minimum_amount_out (u64 LE)
/// ```
pub fn build_swap_ix(
    pool_id: AccountId,
    pool: &RaydiumAmmPool,
    swap_cfg: &RaydiumAmmSwapConfig,
    params: &SwapParams,
    wallet: &mut Wallet,
) -> Result<(), TraderError> {
    // Validate mints
    let (a_to_b, _) = if params.input_mint == pool.coin_mint && params.output_mint == pool.pc_mint {
        (true, ())
    } else if params.input_mint == pool.pc_mint && params.output_mint == pool.coin_mint {
        (false, ())
    } else {
        return Err(TraderError::WrongMints);
    };
    let _ = a_to_b; // direction encoded implicitly by source/destination accounts

    let resolve = |id: AccountId| -> Result<Pubkey, TraderError> {
        pubkey_from_account_id(&id).ok_or(TraderError::PubkeyResolutionFailed(id))
    };

    let pool_pk = resolve(pool_id)?;
    let open_orders_pk = resolve(pool.open_orders)?;
    let target_orders_pk = resolve(pool.target_orders)?;
    let coin_vault_pk = resolve(pool.coin_vault)?;
    let pc_vault_pk = resolve(pool.pc_vault)?;
    let market_program_pk = resolve(pool.market_program)?;
    let market_id_pk = resolve(pool.market_id)?;
    let market_bids_pk = resolve(swap_cfg.market_bids)?;
    let market_asks_pk = resolve(swap_cfg.market_asks)?;
    let market_event_queue_pk = resolve(swap_cfg.market_event_queue)?;
    let market_coin_vault_pk = resolve(swap_cfg.market_coin_vault)?;
    let market_pc_vault_pk = resolve(swap_cfg.market_pc_vault)?;
    let market_vault_signer_pk = resolve(swap_cfg.market_vault_signer)?;
    let user_source_pk = resolve(params.user_source_token_account)?;
    let user_dest_pk = resolve(params.user_destination_token_account)?;
    let user_wallet_pk = resolve(params.user_wallet)?;

    // Build instruction data
    let mut data = [0u8; 17];
    data[0] = 9; // SwapBaseIn discriminant
    data[1..9].copy_from_slice(&params.amount_in.to_le_bytes());
    data[9..17].copy_from_slice(&params.min_amount_out.to_le_bytes());

    // Account list (order matches Raydium AMM v4 program expectation)
    let accounts = vec![
        AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),
        AccountMeta::new(pool_pk, false),
        AccountMeta::new_readonly(RAYDIUM_AMM_AUTHORITY, false),
        AccountMeta::new(open_orders_pk, false),
        AccountMeta::new(target_orders_pk, false),
        AccountMeta::new(coin_vault_pk, false),
        AccountMeta::new(pc_vault_pk, false),
        AccountMeta::new_readonly(market_program_pk, false),
        AccountMeta::new(market_id_pk, false),
        AccountMeta::new(market_bids_pk, false),
        AccountMeta::new(market_asks_pk, false),
        AccountMeta::new(market_event_queue_pk, false),
        AccountMeta::new(market_coin_vault_pk, false),
        AccountMeta::new(market_pc_vault_pk, false),
        AccountMeta::new_readonly(market_vault_signer_pk, false),
        AccountMeta::new(user_source_pk, false),
        AccountMeta::new(user_dest_pk, false),
        AccountMeta::new_readonly(user_wallet_pk, true),
    ];

    wallet.require_signer(params.user_wallet);
    wallet.append_ix(
        Instruction {
            program_id: RAYDIUM_AMM_PROGRAM_ID,
            accounts,
            data: data.to_vec(),
        },
        RAYDIUM_AMM_SWAP_CU,
    );

    Ok(())
}
