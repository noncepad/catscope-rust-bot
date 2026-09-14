//! Sanctum S Controller (INF / Infinity) trader.
//!
//! # Program
//!
//! The Sanctum S Controller (`5ocnV1qiCgaQR8Jb8xWnVbApfaygJ8tNoZfgPwsgx9kx`) is a
//! single pool that accepts many LSTs, routing swaps through SOL as an internal
//! unit of account.  Accounts are **not** Anchor — they are identified by comparing
//! pubkeys against precomputed PDAs.
//!
//! # Spot price
//!
//! Each `LstState` entry records `sol_value` — the total SOL value of that LST's
//! pool reserves.  The pool reserves token account holds `reserve` raw LST units.
//! The implicit exchange rate src → dst in dst raw units is:
//!
//! ```text
//! price = (sol_value_src / reserve_src) / (sol_value_dst / reserve_dst)
//!       = (sol_value_src × reserve_dst) / (sol_value_dst × reserve_src)
//! ```
//!
//! # swap_exact_in instruction layout
//!
//! ```text
//! data[0]      disc = 1u8
//! data[1]      n_src_value_calc_accs (u8)
//! data[2]      n_dst_value_calc_accs (u8)
//! data[3..7]   src_lst_index (u32 LE)
//! data[7..11]  dst_lst_index (u32 LE)
//! data[11..19] min_amount_out (u64 LE)
//! data[19..27] amount (u64 LE)
//! ```
//!
//! Base accounts (12), then src value calc accounts, then dst value calc accounts.

use std::{collections::HashMap, hash::BuildHasherDefault};

use solana_sdk::{
    clock::Slot,
    message::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use twox_hash::XxHash64;

use crate::{
    catscope::witbot::shooter::{Header, Tokenaccountv1},
    err::CatscopeGuestError,
    graph::{AccountId, Graph, SubscriptionRequest},
    log_warn,
    trader::{
        dex::update::Updater,
        pricegraph::{Hop, Route, TradeRouter},
        types::{DexType, TraderError},
    },
    txview::CatscopeInstructionRead,
    util::{account_id_from_pubkey, pubkey_from_account_id},
    wallet::Wallet,
};

// ─── Program / well-known addresses ──────────────────────────────────────────

pub const S_CONTROLLER_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("5ocnV1qiCgaQR8Jb8xWnVbApfaygJ8tNoZfgPwsgx9kx");

const SPL_TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

// Verified against a real, decoded mainnet swap_exact_in transaction --
// the account list includes this exact address. This constant previously
// held a corrupted value (...LJe1bxr instead of ...LJA8knL, same vanity
// prefix) which made every derived pool_reserves_pk/protocol_fee_acc_pk
// for all 126 LSTs point at nonexistent accounts: reserve token events
// never arrived, reserve stayed 0 forever, spot_price() always returned
// None, and SanctumState::batch_router never emitted a single edge --
// independent of anything in the router-admission/ROUTER_POOLS budget
// work. It would also have made SanctumState::swap() build instructions
// with wrong accounts, failing on-chain if ever invoked.
const ASSOCIATED_TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

// ─── Account layout constants ─────────────────────────────────────────────────

const POOL_STATE_SIZE: usize = 176;
const OFF_PS_TRADING_FEE_BPS: usize = 8;
const OFF_PS_LP_FEE_BPS: usize = 10;

const LST_STATE_SIZE: usize = 80;
const OFF_LS_SOL_VALUE: usize = 8;
const OFF_LS_MINT: usize = 16;

// ─── Per-LST compute budget ───────────────────────────────────────────────────

/// CU budget per `swap_exact_in` — includes value-calculator CPIs.
pub const SANCTUM_SWAP_CU: u32 = 300_000;

// ─── SOL value calculator kinds ────────────────────────────────────────────────
//
// Every LST's `swap_exact_in` CPIs into a per-LST "SOL value calculator"
// program to price it. Verified directly against igneous-labs/S (Sanctum's
// own program source, not guessed): all but `Wsol` implement the same
// `generic_pool_calculator` instruction shape, whose LstToSol/SolToLst
// accounts are `[lst_mint, state, pool_state, pool_program, pool_program_data]`
// (idl/sol-value-calculator-programs/generic_pool_calculator.json). Since
// `lst_mint` is already a base account in `swap_exact_in`, the remaining
// four are exactly what `value_calc_accounts` needs, in that order.
//
// `state` is `find_program_address([b"state"], &sol_value_calculator)` for
// every kind here (generic-pool-calculator-lib/src/pda.rs) -- computed at
// resolve time, not stored. `pool_program`/`pool_program_data` are fixed
// per calculator program (one deployment per kind); for Marinade/Lido
// `pool_state` is *also* fixed (a single global pool each), so those two
// need no external data at all. Only Spl/SanctumSpl/SanctumSplMulti vary
// `pool_state` per LST -- sourced from the vendored sanctum-lst-list.toml
// registry on the optimizer side (see `SanctumLstRaw::pool_state`).
struct GenericPoolCalculator {
    calculator_program: Pubkey,
    pool_program: Pubkey,
    pool_program_data: Pubkey,
    /// `Some` for Marinade/Lido (one global pool); `None` for
    /// Spl/SanctumSpl/SanctumSplMulti (pool_state varies per LST, comes
    /// from the vendored registry instead).
    fixed_pool_state: Option<Pubkey>,
}

const GENERIC_POOL_CALCULATORS: &[GenericPoolCalculator] = &[
    // Spl (third-party SPL Stake Pool LSTs, e.g. jitoSOL/bSOL)
    GenericPoolCalculator {
        calculator_program: Pubkey::from_str_const("sp1V4h2gWorkGhVcazBc22Hfo2f5sd7jcjT4EDPrWFF"),
        pool_program: Pubkey::from_str_const("SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy"),
        pool_program_data: Pubkey::from_str_const("EmiU8AQkB2sswTxVB6aCmsAJftoowZGGDXuytm6X65R3"),
        fixed_pool_state: None,
    },
    // SanctumSpl (Sanctum-operated SPL Stake Pool fork; the majority of
    // Sanctum-native LSTs)
    GenericPoolCalculator {
        calculator_program: Pubkey::from_str_const("sspUE1vrh7xRoXxGsg7vR1zde2WdGtJRbyK9uRumBDy"),
        pool_program: Pubkey::from_str_const("SP12tWFxD9oJsVWNavTTBZvMbA6gkAmxtVgxdqvyvhY"),
        pool_program_data: Pubkey::from_str_const("Cn5fegqLh8Fmvffisr4Wk3LmuaUgMMzTFfEuidpZFsvV"),
        fixed_pool_state: None,
    },
    // SanctumSplMulti (Sanctum-operated multi-validator SPL Stake Pool fork)
    GenericPoolCalculator {
        calculator_program: Pubkey::from_str_const("ssmbu3KZxgonUtjEMCKspZzxvUQCxAFnyh1rcHUeEDo"),
        pool_program: Pubkey::from_str_const("SPMBzsVUuoHA4Jm6KunbsotaahvVikZs1JyTW6iJvbn"),
        pool_program_data: Pubkey::from_str_const("HxBTMuB7cFBPVWVJjTi9iBF8MPd7mfY1QnrrWfLAySFd"),
        fixed_pool_state: None,
    },
    // Marinade (mSOL) -- single global pool, no registry lookup needed.
    GenericPoolCalculator {
        calculator_program: Pubkey::from_str_const("mare3SCyfZkAndpBRBeonETmkCCB3TJTTrz8ZN2dnhP"),
        pool_program: Pubkey::from_str_const("MarBmsSgKXdrN1egZf5sqe1TMai9K1rChYNDJgjq7aD"),
        pool_program_data: Pubkey::from_str_const("4PQH9YmfuKrVyZaibkLYpJZPv2FPaybhq2GAuBcWMSBf"),
        fixed_pool_state: Some(Pubkey::from_str_const(
            "8szGkuLTAux9XMgZ2vtY39jVSowEcpBfFfD8hXSEqdGC",
        )),
    },
    // Lido (stSOL) -- single global pool, no registry lookup needed.
    GenericPoolCalculator {
        calculator_program: Pubkey::from_str_const("1idUSy4MGGKyKhvjSnGZ6Zc7Q4eKQcibym4BkEEw9KR"),
        pool_program: Pubkey::from_str_const("CrX7kMhLC3cSsXJdT7JDgqrRVWGnUpX3gfEfxxU2NVLi"),
        pool_program_data: Pubkey::from_str_const("CHZNLhDXKrsXBmmv947RFciquwBsn2NdABmhpxoX3wgZ"),
        fixed_pool_state: Some(Pubkey::from_str_const(
            "49Yi1TKkNyYjPAFdR9LBvoHcUjuPX4Df5T5yv39w2XTn",
        )),
    },
];

/// wSOL's calculator is a trivial 1:1 identity and needs zero extra
/// accounts beyond `lst_mint` (already a base `swap_exact_in` account) --
/// unlike the other 5 kinds, it doesn't implement the
/// `generic_pool_calculator` shape at all.
const WSOL_CALCULATOR_PROGRAM: Pubkey =
    Pubkey::from_str_const("wsoGmxQLSvwWpuaidCApxN5kEowLe2HLQLJhCQnj4bE");

/// Resolve the `value_calc_accounts` for one LST, given its calculator
/// program and (for Spl/SanctumSpl/SanctumSplMulti) its per-LST
/// `pool_state` from the vendored registry. Returns an empty vec --
/// identical to never implementing this at all -- when the calculator
/// isn't one of the 6 known kinds, or (Spl-family only) the registry
/// didn't have a `pool_state` for this LST: a documented gap, not a guess.
fn resolve_value_calc_accounts(
    sol_value_calculator: Pubkey,
    pool_state: Option<Pubkey>,
) -> Vec<Pubkey> {
    if sol_value_calculator == WSOL_CALCULATOR_PROGRAM {
        return vec![];
    }
    let Some(calc) = GENERIC_POOL_CALCULATORS
        .iter()
        .find(|c| c.calculator_program == sol_value_calculator)
    else {
        return vec![];
    };
    let Some(pool_state) = calc.fixed_pool_state.or(pool_state) else {
        return vec![];
    };
    let state_pda = Pubkey::find_program_address(&[b"state"], &sol_value_calculator).0;
    vec![
        state_pda,
        pool_state,
        calc.pool_program,
        calc.pool_program_data,
    ]
}

// ─── Setup types (provided by the caller at startup) ─────────────────────────

/// Configuration for one LST in the Sanctum pool.
#[derive(Clone)]
pub struct SanctumLstSetup {
    pub mint: Pubkey,
    pub sol_value_calculator: Pubkey,
    /// Additional accounts required by the value-calculator CPI for this LST.
    pub value_calc_accounts: Vec<Pubkey>,
    /// Build-time snapshot of this LST's `sol_value` (from `prefetch.db`'s
    /// `sanctum_lst` table) -- seeds [`LstInfo::sol_value`] so `spot_price()`
    /// doesn't have to wait for the first live `lst_state_list` account
    /// update before it can resolve. Not real-time; overwritten by the
    /// first live update same as before, just no longer starts at 0.
    pub sol_value: u64,
    /// Build-time snapshot of this LST's pool-reserves ATA balance (from
    /// `prefetch.db`'s `sanctum_lst.reserve` column, fetched by the Go
    /// `optimizer/prefetch/sanctum` package). Seeds [`LstInfo::reserve`]
    /// the same way `sol_value` above does -- overwritten by the first
    /// live `on_token` event, just no longer starts at 0.
    pub reserve: u64,
}

/// Configuration for the Sanctum S Controller pool.
#[derive(Clone)]
pub struct SanctumSetup {
    pub program_id: Pubkey,
    pub lsts: Vec<SanctumLstSetup>,
}

// ─── Internal per-LST live state ──────────────────────────────────────────────

#[derive(Debug)]
struct LstInfo {
    mint_id: AccountId,
    mint_pk: Pubkey,
    #[allow(dead_code)]
    sol_value_calculator_id: AccountId,
    value_calc_account_ids: Vec<AccountId>,
    /// ATA(pool_state_pda, mint_pk) — pool's reserve vault for this LST.
    pool_reserves_id: AccountId,
    pool_reserves_pk: Pubkey,
    /// ATA(protocol_fee_pda, mint_pk) — fee accumulator (used when this LST is dst).
    protocol_fee_acc_pk: Pubkey,
    /// SOL value of this LST's pool reserves, updated from `lst_state_list`.
    sol_value: u64,
    /// Raw token balance of the pool reserves vault, updated from token events.
    reserve: u64,
}

// ─── SanctumState ─────────────────────────────────────────────────────────────

/// Live state for the Sanctum S Controller pool.
///
/// Implements [`Updater`] like the other dex states: [`Updater::on_account`]
/// for every account update the host delivers, [`Updater::on_token`] for
/// every SPL token account event, and [`Updater::batch_router`] to add all
/// LST pairs to a [`TradeRouter`] for arbitrage detection.
#[derive(Debug)]
pub struct SanctumState {
    program_id: AccountId,
    pool_state_pda: Pubkey,
    pub pool_state_id: AccountId,
    lst_state_list_pda: Pubkey,
    pub lst_state_list_id: AccountId,
    protocol_fee_pda: Pubkey,
    trading_fee_bps: u16,
    lp_fee_bps: u16,
    lsts: Vec<LstInfo>,
    /// pool_reserves_id → index into `lsts`.
    reserve_to_lst: HashMap<AccountId, usize>,
    m_vault: HashMap<AccountId, Tokenaccountv1, BuildHasherDefault<XxHash64>>,
    /// TEMP diagnostic: log the first `lst_state_list` account delivery
    /// (size + alignment against `LST_STATE_SIZE`) exactly once, to check
    /// whether the fixed 80-byte/no-discriminator layout this parser
    /// assumes (mirroring optimizer/prefetch/sanctum/lst.go, which is
    /// known-correct since it's what populated sol_value in prefetch.db)
    /// actually matches what the live account delivers -- sol_value
    /// staying 0 forever would explain spot_price() never resolving and
    /// batch_router never emitting a Sanctum edge, independent of the
    /// ROUTER_POOLS admission fix.
    lst_state_list_logged: bool,
    /// TEMP diagnostic: see [`Self::lst_state_list_logged`] -- pairs with
    /// it to confirm reserve-vault token events are actually arriving.
    first_reserve_logged: bool,
}

/// Builds a [`SanctumSetup`] from the build-time-embedded LST list
/// (`sanctum_config::SANCTUM_LSTS`). `value_calc_accounts` is resolved via
/// [`resolve_value_calc_accounts`] against the 6 known SOL-value-calculator
/// kinds -- empty for any LST whose calculator isn't one of those 6, or
/// (Spl-family only) whose `pool_state` wasn't in the vendored registry.
fn default_setup() -> SanctumSetup {
    SanctumSetup {
        program_id: S_CONTROLLER_PROGRAM_ID,
        lsts: crate::sanctum_config::SANCTUM_LSTS
            .iter()
            .map(|lst| {
                let sol_value_calculator = Pubkey::new_from_array(lst.sol_value_calculator);
                let pool_state = if lst.pool_state == [0u8; 32] {
                    None
                } else {
                    Some(Pubkey::new_from_array(lst.pool_state))
                };
                SanctumLstSetup {
                    mint: Pubkey::new_from_array(lst.mint),
                    sol_value_calculator,
                    value_calc_accounts: resolve_value_calc_accounts(
                        sol_value_calculator,
                        pool_state,
                    ),
                    sol_value: lst.sol_value,
                    reserve: lst.reserve,
                }
            })
            .collect(),
    }
}

impl SanctumState {
    /// Builds this dex's live state and returns its pending subscription
    /// requests alongside it -- doesn't subscribe itself. See
    /// `dex::raydium::amm::RaydiumAmm::new`'s doc comment for why (paced
    /// through a shared [`crate::graph::SubscriptionQueue`] owned by
    /// `DexState` instead).
    pub fn new() -> (Self, Vec<SubscriptionRequest>) {
        let setup = default_setup();
        let program_id_pk = setup.program_id;
        let mut l_req = Vec::with_capacity(setup.lsts.len());
        let pool_state_pda_pk = Pubkey::find_program_address(&[b"state"], &program_id_pk).0;
        let pool_state_id = account_id_from_pubkey(&pool_state_pda_pk);
        l_req.push(SubscriptionRequest {
            root: pool_state_id,
            filter_weight: 0,
            depth: 1,
        });
        let lst_state_list_pda_pk =
            Pubkey::find_program_address(&[b"lst-state-list"], &program_id_pk).0;
        let lst_state_list_id = account_id_from_pubkey(&lst_state_list_pda_pk);
        l_req.push(SubscriptionRequest {
            root: lst_state_list_id,
            filter_weight: 0,
            depth: 1,
        });
        let protocol_fee_pda_pk =
            Pubkey::find_program_address(&[b"protocol-fee"], &program_id_pk).0;
        let protocol_fee_pda_id = account_id_from_pubkey(&pool_state_pda_pk);
        l_req.push(SubscriptionRequest {
            root: protocol_fee_pda_id,
            filter_weight: 0,
            depth: 1,
        });

        let mut lsts = Vec::with_capacity(setup.lsts.len());
        let mut reserve_to_lst: HashMap<AccountId, usize> = HashMap::new();

        let mut m_vault =
            HashMap::with_capacity_and_hasher(setup.lsts.len() * 2, BuildHasherDefault::default());

        for lst_setup in setup.lsts {
            let mint_id = account_id_from_pubkey(&lst_setup.mint);
            let sol_value_calculator_id = account_id_from_pubkey(&lst_setup.sol_value_calculator);
            let value_calc_account_ids = lst_setup
                .value_calc_accounts
                .iter()
                .map(|pk| account_id_from_pubkey(pk))
                .collect();

            let pool_reserves_pk =
                spl_ata(&pool_state_pda_pk, &SPL_TOKEN_PROGRAM_ID, &lst_setup.mint);
            let pool_reserves_id = account_id_from_pubkey(&pool_reserves_pk);
            l_req.push(SubscriptionRequest {
                root: pool_reserves_id,
                filter_weight: 0,
                depth: 1,
            });
            m_vault.insert(
                pool_reserves_id,
                Tokenaccountv1 {
                    id: pool_reserves_id,
                    owner: 0,
                    mint: 0,
                    amount: 0,
                    slot: 0,
                    version: 0,
                },
            );

            let protocol_fee_acc_pk =
                spl_ata(&protocol_fee_pda_pk, &SPL_TOKEN_PROGRAM_ID, &lst_setup.mint);
            let protocol_fee_acc_id = account_id_from_pubkey(&protocol_fee_acc_pk);
            l_req.push(SubscriptionRequest {
                root: protocol_fee_acc_id,
                filter_weight: 0,
                depth: 1,
            });
            m_vault.insert(
                protocol_fee_acc_id,
                Tokenaccountv1 {
                    id: pool_reserves_id,
                    owner: 0,
                    mint: 0,
                    amount: 0,
                    slot: 0,
                    version: 0,
                },
            );
            let idx = lsts.len();
            reserve_to_lst.insert(pool_reserves_id, idx);
            lsts.push(LstInfo {
                mint_id,
                mint_pk: lst_setup.mint,
                sol_value_calculator_id,
                value_calc_account_ids,
                pool_reserves_id,
                pool_reserves_pk,
                protocol_fee_acc_pk,
                sol_value: lst_setup.sol_value,
                reserve: lst_setup.reserve,
            });
        }
        let state = Self {
            m_vault,
            program_id: account_id_from_pubkey(&program_id_pk),
            pool_state_pda: pool_state_pda_pk,
            pool_state_id,
            lst_state_list_pda: lst_state_list_pda_pk,
            lst_state_list_id,
            protocol_fee_pda: protocol_fee_pda_pk,
            trading_fee_bps: 0,
            lp_fee_bps: 0,
            lsts,
            reserve_to_lst,
            lst_state_list_logged: false,
            first_reserve_logged: false,
        };
        (state, l_req)
    }

    #[inline]
    pub fn program_id(&self) -> &AccountId {
        &self.program_id
    }

    pub fn lst_count(&self) -> usize {
        self.lsts.len()
    }

    /// Account IDs that should be subscribed to via the Catscope graph.
    ///
    /// Pass the returned IDs to your subscription request at startup so the host
    /// delivers updates to [`Updater::on_account`] and [`Updater::on_token`].
    pub fn account_ids(&self) -> Vec<AccountId> {
        let mut ids = vec![self.pool_state_id, self.lst_state_list_id];
        for lst in &self.lsts {
            ids.push(lst.pool_reserves_id);
        }
        ids
    }

    fn parse_pool_state(&mut self, data: &[u8]) {
        if data.len() < POOL_STATE_SIZE {
            return;
        }
        self.trading_fee_bps = u16::from_le_bytes(
            data[OFF_PS_TRADING_FEE_BPS..OFF_PS_TRADING_FEE_BPS + 2]
                .try_into()
                .unwrap(),
        );
        self.lp_fee_bps = u16::from_le_bytes(
            data[OFF_PS_LP_FEE_BPS..OFF_PS_LP_FEE_BPS + 2]
                .try_into()
                .unwrap(),
        );
    }

    fn parse_lst_state_list(&mut self, data: &[u8]) {
        if !self.lst_state_list_logged {
            self.lst_state_list_logged = true;
            log_warn!(
                "sanctum: lst_state_list delivered: len={} lst_state_size={} \
                 len%size={} n_entries={} known_lsts={}",
                data.len(),
                LST_STATE_SIZE,
                data.len() % LST_STATE_SIZE,
                data.len() / LST_STATE_SIZE,
                self.lsts.len(),
            );
        }
        if data.is_empty() || data.len() % LST_STATE_SIZE != 0 {
            return;
        }
        let n = data.len() / LST_STATE_SIZE;
        for i in 0..n {
            let base = i * LST_STATE_SIZE;
            let mint_bytes: [u8; 32] = data[base + OFF_LS_MINT..base + OFF_LS_MINT + 32]
                .try_into()
                .unwrap();
            let mint_pk = Pubkey::from(mint_bytes);
            let sol_value = u64::from_le_bytes(
                data[base + OFF_LS_SOL_VALUE..base + OFF_LS_SOL_VALUE + 8]
                    .try_into()
                    .unwrap(),
            );
            if let Some(lst) = self.lsts.iter_mut().find(|l| l.mint_pk == mint_pk) {
                lst.sol_value = sol_value;
            }
        }
    }

    /// Spot exchange rate: dst raw units per src raw unit (before fees).
    ///
    /// Returns `None` when reserves or sol_values are not yet known.
    pub fn spot_price(&self, src_mint_id: AccountId, dst_mint_id: AccountId) -> Option<f64> {
        let src = self.lsts.iter().find(|l| l.mint_id == src_mint_id)?;
        let dst = self.lsts.iter().find(|l| l.mint_id == dst_mint_id)?;
        if src.reserve == 0 || dst.reserve == 0 || src.sol_value == 0 || dst.sol_value == 0 {
            return None;
        }
        let price = (src.sol_value as f64 * dst.reserve as f64)
            / (dst.sol_value as f64 * src.reserve as f64);
        if price > 0.0 {
            Some(price)
        } else {
            None
        }
    }

    /// Total swap fee in basis points (trading + LP).
    pub fn total_fee_bps(&self) -> u16 {
        self.trading_fee_bps.saturating_add(self.lp_fee_bps)
    }

    /// Re-derive and upsert one directed LST pair `i -> j`. `price` is
    /// deliberately `unwrap_or(0.0)` rather than skipped on `None` --
    /// `add_generic_pair`'s own `price_b_per_a <= 0.0` gate then removes
    /// any stale edge for a pair that used to be priced (e.g. a reserve
    /// legitimately dropping to zero) but no longer is.
    fn upsert_pair(&self, i: usize, j: usize, fee_frac: f64, router: &mut TradeRouter) {
        let src = &self.lsts[i];
        let dst = &self.lsts[j];
        let price = self.spot_price(src.mint_id, dst.mint_id).unwrap_or(0.0);
        router.add_generic_pair(
            self.pool_state_id,
            src.mint_id,
            dst.mint_id,
            price,
            fee_frac,
            src.reserve,
            dst.reserve,
            DexType::Sanctum,
        );
    }

    /// Re-derive every LST pair -- O(n^2), used when `pool_state_id`
    /// (fee rates) or `lst_state_list_id` (every LST's `sol_value`)
    /// changes, since either invalidates every pair at once. Infrequent
    /// (global config updates), and still cheap relative to the
    /// ~8000-pool full `batch_router` rebuild it replaces for the
    /// incremental path.
    fn upsert_all_pairs(&mut self, router: &mut TradeRouter) {
        let fee_frac = self.total_fee_bps() as f64 / 10_000.0;
        let n = self.lsts.len();
        for i in 0..n {
            for j in 0..n {
                if i != j {
                    self.upsert_pair(i, j, fee_frac, router);
                }
            }
        }
    }

    /// Re-derive just the `2*(n-1)` pairs touching LST `idx` -- correct
    /// since one LST's reserve changing affects every pair it's in, and
    /// O(n) rather than `upsert_all_pairs`'s O(n^2).
    fn upsert_lst_pairs(&mut self, idx: usize, router: &mut TradeRouter) {
        let fee_frac = self.total_fee_bps() as f64 / 10_000.0;
        let n = self.lsts.len();
        for j in 0..n {
            if j != idx {
                self.upsert_pair(idx, j, fee_frac, router);
                self.upsert_pair(j, idx, fee_frac, router);
            }
        }
    }

    /// Build a `swap_exact_in` instruction and append it to `wallet`.
    pub fn swap(
        &self,
        src_mint_id: AccountId,
        dst_mint_id: AccountId,
        amount_in: u64,
        min_amount_out: u64,
        user_wallet: AccountId,
        user_src_token_account: AccountId,
        user_dst_token_account: AccountId,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let src_idx = self
            .lsts
            .iter()
            .position(|l| l.mint_id == src_mint_id)
            .ok_or(TraderError::WrongMints)?;
        let dst_idx = self
            .lsts
            .iter()
            .position(|l| l.mint_id == dst_mint_id)
            .ok_or(TraderError::WrongMints)?;
        let src = &self.lsts[src_idx];
        let dst = &self.lsts[dst_idx];

        let resolve = |id: AccountId| -> Result<Pubkey, TraderError> {
            pubkey_from_account_id(&id).ok_or(TraderError::PubkeyResolutionFailed(id))
        };

        let user_wallet_pk = resolve(user_wallet)?;
        let user_src_pk = resolve(user_src_token_account)?;
        let user_dst_pk = resolve(user_dst_token_account)?;

        let n_src = src.value_calc_account_ids.len() as u8;
        let n_dst = dst.value_calc_account_ids.len() as u8;

        // 27-byte data: disc(1) + n_src(1) + n_dst(1) + src_idx(4) + dst_idx(4) + min_out(8) + amount(8)
        let mut data = Vec::with_capacity(27);
        data.push(1u8);
        data.push(n_src);
        data.push(n_dst);
        data.extend_from_slice(&(src_idx as u32).to_le_bytes());
        data.extend_from_slice(&(dst_idx as u32).to_le_bytes());
        data.extend_from_slice(&min_amount_out.to_le_bytes());
        data.extend_from_slice(&amount_in.to_le_bytes());

        let mut accounts = vec![
            AccountMeta::new(user_wallet_pk, true),           // signer
            AccountMeta::new_readonly(src.mint_pk, false),    // src_lst_mint
            AccountMeta::new_readonly(dst.mint_pk, false),    // dst_lst_mint
            AccountMeta::new(user_src_pk, false),             // src_lst_acc
            AccountMeta::new(user_dst_pk, false),             // dst_lst_acc
            AccountMeta::new(dst.protocol_fee_acc_pk, false), // protocol_fee_accumulator
            AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false), // src_lst_token_program
            AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false), // dst_lst_token_program
            AccountMeta::new(self.pool_state_pda, false),     // pool_state
            AccountMeta::new(self.lst_state_list_pda, false), // lst_state_list
            AccountMeta::new(src.pool_reserves_pk, false),    // src_pool_reserves
            AccountMeta::new(dst.pool_reserves_pk, false),    // dst_pool_reserves
        ];

        for &id in &src.value_calc_account_ids {
            accounts.push(AccountMeta::new_readonly(resolve(id)?, false));
        }
        for &id in &dst.value_calc_account_ids {
            accounts.push(AccountMeta::new_readonly(resolve(id)?, false));
        }

        wallet.require_signer(user_wallet);
        wallet.append_ix(
            Instruction {
                program_id: S_CONTROLLER_PROGRAM_ID,
                accounts,
                data,
            },
            SANCTUM_SWAP_CU,
        );
        Ok(())
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
        self.swap(hop.input_mint, hop.output_mint, hop.amount_in, hop.amount_out, owner, source_ata, dest_ata, wallet)
    }
}

impl Updater for SanctumState {
    fn on_account(&mut self, header: &Header, body: &[u8]) {
        if self.program_id != header.owner {
            return;
        }
        if header.accountid == self.pool_state_id {
            self.parse_pool_state(body);
        } else if header.accountid == self.lst_state_list_id {
            self.parse_lst_state_list(body);
        }
    }

    /// Process an SPL token account event to update pool reserve balances.
    fn on_token(&mut self, ta: &Tokenaccountv1) -> bool {
        let idx = match self.reserve_to_lst.get(&ta.id) {
            Some(&idx) => idx,
            None => return false,
        };
        // TEMP diagnostic: confirm at least one reserve-vault token event
        // is actually being delivered (paired with the lst_state_list
        // one-shot log above -- see that field's doc comment).
        if !self.first_reserve_logged {
            self.first_reserve_logged = true;
            log_warn!(
                "sanctum: first reserve token event: mint={} amount={} \
                 lst_state_list_seen={}",
                self.lsts[idx].mint_pk,
                ta.amount,
                self.lst_state_list_logged,
            );
        }
        self.lsts[idx].reserve = ta.amount;
        true
    }

    /// Add all LST→LST pairs to `router` for arbitrage detection.
    ///
    /// [`pool_state_id`](Self::pool_state_id) is used as the pool identifier for
    /// every edge so that [`SanctumState::swap`] can reconstruct the instruction
    /// from the hop's input/output mints.
    fn batch_router(&mut self, router: &mut TradeRouter) {
        self.upsert_all_pairs(router);
    }

    /// `pool_state_id`/`lst_state_list_id` are global -- either changing
    /// invalidates every LST pair at once, so this re-derives all of them
    /// (`upsert_all_pairs`, O(n^2)) rather than just one.
    fn refresh_account_router(&mut self, account_id: AccountId, router: &mut TradeRouter) {
        if account_id == self.pool_state_id || account_id == self.lst_state_list_id {
            self.upsert_all_pairs(router);
        }
    }

    fn refresh_token_router(&mut self, ta_id: AccountId, router: &mut TradeRouter) {
        if let Some(&idx) = self.reserve_to_lst.get(&ta_id) {
            self.upsert_lst_pairs(idx, router);
        }
    }

    fn on_tx(&mut self, _ix: &CatscopeInstructionRead<'_>, _slot: &Slot) {}

    fn flush_pool(&mut self, _g: &Graph, _max_per_flush: usize) -> Result<(), CatscopeGuestError> {
        Ok(())
    }
}

// ─── Arbitrage detection ──────────────────────────────────────────────────────

/// Which leg of the two-hop cycle goes through Sanctum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SanctumArbDirection {
    /// Sanctum executes first (start_mint → mid_mint), then the external route
    /// closes the loop (mid_mint → start_mint).
    SanctumFirst,
    /// The external route goes first (start_mint → mid_mint), then Sanctum
    /// closes the loop (mid_mint → start_mint).
    SanctumSecond,
}

/// A profitable two-hop arbitrage cycle involving Sanctum.
#[derive(Debug, Clone)]
pub struct SanctumArb {
    /// Token the cycle starts and ends with.
    pub start_mint: AccountId,
    /// Intermediate LST (the token exchanged between the two legs).
    pub mid_mint: AccountId,
    /// Raw units of `start_mint` deployed at the start.
    pub amount_in: u64,
    /// Raw units of `mid_mint` after the first leg.
    pub mid_amount: u64,
    /// Raw units of `start_mint` recovered after the second leg.
    pub amount_out: u64,
    /// Which leg goes through Sanctum.
    pub direction: SanctumArbDirection,
    /// The external multi-hop route (Orca / Raydium).
    pub external_route: Route,
}

impl SanctumArb {
    /// Gross profit in raw units of `start_mint`.
    pub fn profit_raw(&self) -> u64 {
        self.amount_out.saturating_sub(self.amount_in)
    }
}

impl SanctumState {
    /// Approximate constant-rate quote for a Sanctum LST swap.
    ///
    /// Uses `spot_price × (1 − fee)` — ignores price impact, so it is
    /// optimistic for large `amount_in` relative to pool reserves.  Suitable
    /// as a fast pre-filter; validate with the on-chain value calculators before
    /// submitting a transaction.
    pub fn quote(&self, src_mint_id: AccountId, dst_mint_id: AccountId, amount_in: u64) -> u64 {
        let price = match self.spot_price(src_mint_id, dst_mint_id) {
            Some(p) => p,
            None => return 0,
        };
        let after_fee = 1.0 - self.total_fee_bps() as f64 / 10_000.0;
        (amount_in as f64 * price * after_fee) as u64
    }

    /// Detect profitable cross-venue arbitrage between Sanctum and an external router.
    ///
    /// For every ordered LST pair (A, B) registered in this pool, two cycles are
    /// checked:
    ///
    /// - **SanctumFirst**: swap A → B on Sanctum, then route B → A externally.
    /// - **SanctumSecond**: route A → B externally, then swap B → A on Sanctum.
    ///
    /// `external_router` should contain only non-Sanctum pools (Orca, Raydium,
    /// etc.) so the two legs are genuinely distinct venues.  Build it the same
    /// way as the main router but skip [`Updater::batch_router`].
    ///
    /// `max_hops` caps the length of the external leg (2–3 is typical).
    ///
    /// Returns all profitable opportunities sorted by profit descending.
    pub fn find_arb(
        &self,
        external_router: &TradeRouter,
        amount_in: u64,
        max_hops: usize,
    ) -> Vec<SanctumArb> {
        let mut result = Vec::new();

        for i in 0..self.lsts.len() {
            for j in 0..self.lsts.len() {
                if i == j {
                    continue;
                }
                let a = self.lsts[i].mint_id;
                let b = self.lsts[j].mint_id;

                // ── SanctumFirst: A →[Sanctum]→ B →[external]→ A ──────────────
                let sanctum_out = self.quote(a, b, amount_in);
                if sanctum_out > 0 {
                    if let Some(route) = external_router.route(b, a, sanctum_out, max_hops) {
                        let amount_out = route.amount_out();
                        if amount_out > amount_in {
                            result.push(SanctumArb {
                                start_mint: a,
                                mid_mint: b,
                                amount_in,
                                mid_amount: sanctum_out,
                                amount_out,
                                direction: SanctumArbDirection::SanctumFirst,
                                external_route: route,
                            });
                        }
                    }
                }

                // ── SanctumSecond: A →[external]→ B →[Sanctum]→ A ────────────
                if let Some(route) = external_router.route(a, b, amount_in, max_hops) {
                    let ext_out = route.amount_out();
                    if ext_out > 0 {
                        let amount_out = self.quote(b, a, ext_out);
                        if amount_out > amount_in {
                            result.push(SanctumArb {
                                start_mint: a,
                                mid_mint: b,
                                amount_in,
                                mid_amount: ext_out,
                                amount_out,
                                direction: SanctumArbDirection::SanctumSecond,
                                external_route: route,
                            });
                        }
                    }
                }
            }
        }

        result.sort_by(|a, b| b.profit_raw().cmp(&a.profit_raw()));
        result
    }
}

/// Derive an SPL associated token address.
fn spl_ata(wallet: &Pubkey, token_program: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[wallet.as_ref(), token_program.as_ref(), mint.as_ref()],
        &ASSOCIATED_TOKEN_PROGRAM_ID,
    )
    .0
}
