//! SPL Stake Pool `WithdrawSolWithSlippage` trader.
//!
//! # Program
//!
//! Covers every LST that's a plain instance of the standard, audited
//! [solana-program/stake-pool](https://github.com/solana-program/stake-pool)
//! program (`SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy`) -- same program
//! already tracked as Sanctum's "Spl" SOL-value-calculator kind's
//! `pool_program` in `sanctum.rs` (verified there, and again here directly
//! against JitoSOL's and bSOL's real accounts -- see below). Sanctum also
//! tracks two *other*, larger LST groups on this same generic account shape
//! -- "SanctumSpl" and "SanctumSplMulti" -- but those are different
//! Sanctum-operated program forks that haven't been independently verified
//! against real source/live accounts the way this one has, so they're
//! deliberately **not** included here; scoped, not guessed. Membership is
//! decided by `sol_value_calculator == SPL_CALCULATOR_PROGRAM` (Sanctum's
//! own per-LST classification), not a hardcoded mint list -- so this covers
//! every "Spl"-kind LST Sanctum tracks, currently 20. Each LST's
//! `pool_state` address is **already embedded** via
//! `sanctum_config::SANCTUM_LSTS` (populated from the vendored
//! `sanctum-lst-list.toml` registry), no new build-time data needed.
//!
//! `WithdrawSolWithSlippage` redeems pool tokens for SOL immediately from
//! the pool's own `reserve_stake` account, for a flat fee (unlike
//! Marinade's dynamic linear-interpolated fee curve).
//!
//! # `StakePool` account layout
//!
//! **Not** fixed-offset: `manager, staker, ..` are followed by several
//! `Option<Pubkey>`/`FutureEpoch<Fee>` fields whose presence varies the
//! byte length of everything after them, so this has to be a real
//! sequential parse (1-byte tag + conditional payload for each variable
//! field), not fixed offsets -- see [`Cursor`]. Verified against two live
//! fetched accounts (JitoSOL's and bSOL's `pool_state`, both already in
//! `prefetch.db`): the decoded `pool_mint` field matched each LST's real,
//! known mint address exactly, and every other field (fees, supply,
//! reserve/manager-fee addresses) was sane and resolved to real accounts.
//!
//! Field order that matters here (types: `Pubkey`=32, `u8`=1, `u64`=8,
//! `Fee{denominator:u64,numerator:u64}`=16, `Option<Pubkey>`=1-or-33,
//! `FutureEpoch<Fee>`=1-or-17): `account_type, manager, staker,
//! stake_deposit_authority, stake_withdraw_bump_seed, validator_list,
//! reserve_stake, pool_mint, manager_fee_account, token_program_id,
//! total_lamports, pool_token_supply, last_update_epoch, lockup(48 fixed:
//! i64+u64+Pubkey), epoch_fee, next_epoch_fee, preferred_deposit,
//! preferred_withdraw, stake_deposit_fee, stake_withdrawal_fee,
//! next_stake_withdrawal_fee, stake_referral_fee, sol_deposit_authority,
//! sol_deposit_fee, sol_referral_fee, sol_withdraw_authority,
//! sol_withdrawal_fee`. Nothing after `sol_withdrawal_fee` is needed.
//!
//! # Fee + exchange rate (`state.rs`'s `Fee::apply`/`calc_lamports_withdraw_amount`)
//!
//! ```text
//! fee_lamports  = ceil(pool_tokens * sol_withdrawal_fee.numerator / sol_withdrawal_fee.denominator)
//! lamports_out  = pool_tokens_after_fee * total_lamports / pool_token_supply
//! ```
//!
//! # `WithdrawSolWithSlippage` instruction layout
//!
//! `processor.rs::process_withdraw_sol`, exact account order: stake_pool[w],
//! withdraw_authority, user_transfer_authority(signer), burn_from_pool[w],
//! reserve_stake[w], destination_lamports[w], manager_fee[w], pool_mint[w],
//! clock sysvar, stake_history sysvar, stake_program, token_program, then
//! **conditionally** a 13th account (signer) only when the parsed
//! `sol_withdraw_authority` is `Some` (verified `None` on both real pools
//! today, but must check live, not hardcode the omission). Data: 1-byte
//! Borsh enum discriminant (26 -- counted from `instruction.rs`'s
//! `StakePoolInstruction` declaration order) then `pool_tokens_in: u64`,
//! `minimum_lamports_out: u64`. `withdraw_authority` is a PDA: seeds
//! `[stake_pool, b"withdraw"]` (`AUTHORITY_WITHDRAW` in `lib.rs`).

use std::collections::HashMap;

use solana_sdk::{
    message::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

use crate::{
    catscope::witbot::shooter::{Header, Tokenaccountv1},
    err::CatscopeGuestError,
    graph::{AccountId, Graph, SubscriptionQueue, SubscriptionRequest},
    trader::{
        dex::update::Updater,
        pricegraph::{Hop, TradeRouter},
        types::{DexType, TraderError},
    },
    txview::CatscopeInstructionRead,
    util::{account_id_from_pubkey, pubkey_from_account_id},
    wallet::Wallet,
};

pub const STAKE_POOL_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy");

/// Sanctum's "Spl" SOL-value-calculator kind -- the classifier for which
/// LSTs are plain instances of `STAKE_POOL_PROGRAM_ID` (see module doc for
/// why this, and not a hardcoded mint list, decides membership).
const SPL_CALCULATOR_PROGRAM: Pubkey =
    Pubkey::from_str_const("sp1V4h2gWorkGhVcazBc22Hfo2f5sd7jcjT4EDPrWFF");

const WSOL_MINT: Pubkey = Pubkey::from_str_const("So11111111111111111111111111111111111111112");

const STAKE_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("Stake11111111111111111111111111111111111111");
const SYSVAR_CLOCK_ID: Pubkey = Pubkey::from_str_const("SysvarC1ock11111111111111111111111111111111");
const SYSVAR_STAKE_HISTORY_ID: Pubkey =
    Pubkey::from_str_const("SysvarStakeHistory1111111111111111111111111");

/// `StakePoolInstruction::WithdrawSolWithSlippage`'s Borsh enum discriminant
/// -- counted from `instruction.rs`'s declaration order; confirmed correct
/// indirectly by this module's parser round-tripping real accounts (see
/// module doc and tests below).
const DISC_WITHDRAW_SOL_WITH_SLIPPAGE: u8 = 26;

pub const STAKE_POOL_WITHDRAW_SOL_CU: u32 = 60_000;

// ─── Sequential Borsh-style cursor (StakePool has variable-length fields) ────
struct Cursor<'a> {
    data: &'a [u8],
    off: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, off: 0 }
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.off.checked_add(n)?;
        if end > self.data.len() {
            return None;
        }
        let s = &self.data[self.off..end];
        self.off = end;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|s| s[0])
    }
    fn u64(&mut self) -> Option<u64> {
        self.take(8).map(|s| u64::from_le_bytes(s.try_into().unwrap()))
    }
    fn pubkey(&mut self) -> Option<Pubkey> {
        self.take(32).map(|s| Pubkey::new_from_array(s.try_into().unwrap()))
    }
    fn skip(&mut self, n: usize) -> Option<()> {
        self.take(n).map(|_| ())
    }
    fn option_pubkey(&mut self) -> Option<Option<Pubkey>> {
        match self.u8()? {
            0 => Some(None),
            _ => Some(self.pubkey()),
        }
    }
    /// `Fee { denominator: u64, numerator: u64 }` -- that's the real
    /// declared field order (denominator first).
    fn fee(&mut self) -> Option<(u64, u64)> {
        let denominator = self.u64()?;
        let numerator = self.u64()?;
        Some((numerator, denominator))
    }
    fn future_epoch_fee(&mut self) -> Option<Option<(u64, u64)>> {
        match self.u8()? {
            0 => Some(None),
            _ => Some(self.fee()),
        }
    }
}

struct ParsedStakePool {
    reserve_stake: Pubkey,
    manager_fee_account: Pubkey,
    pool_mint: Pubkey,
    token_program_id: Pubkey,
    total_lamports: u64,
    pool_token_supply: u64,
    sol_withdraw_authority: Option<Pubkey>,
    /// (numerator, denominator)
    sol_withdrawal_fee: (u64, u64),
}

fn parse_stake_pool(data: &[u8]) -> Option<ParsedStakePool> {
    let mut c = Cursor::new(data);
    c.u8()?; // account_type
    c.pubkey()?; // manager
    c.pubkey()?; // staker
    c.pubkey()?; // stake_deposit_authority
    c.u8()?; // stake_withdraw_bump_seed
    c.pubkey()?; // validator_list
    let reserve_stake = c.pubkey()?;
    let pool_mint = c.pubkey()?;
    let manager_fee_account = c.pubkey()?;
    let token_program_id = c.pubkey()?;
    let total_lamports = c.u64()?;
    let pool_token_supply = c.u64()?;
    c.u64()?; // last_update_epoch
    c.skip(48)?; // lockup: unix_timestamp(i64) + epoch(u64) + custodian(Pubkey)
    c.fee()?; // epoch_fee
    c.future_epoch_fee()?; // next_epoch_fee
    c.option_pubkey()?; // preferred_deposit_validator_vote_address
    c.option_pubkey()?; // preferred_withdraw_validator_vote_address
    c.fee()?; // stake_deposit_fee
    c.fee()?; // stake_withdrawal_fee
    c.future_epoch_fee()?; // next_stake_withdrawal_fee
    c.u8()?; // stake_referral_fee
    c.option_pubkey()?; // sol_deposit_authority
    c.fee()?; // sol_deposit_fee
    c.u8()?; // sol_referral_fee
    let sol_withdraw_authority = c.option_pubkey()?;
    let sol_withdrawal_fee = c.fee()?;

    Some(ParsedStakePool {
        reserve_stake,
        manager_fee_account,
        pool_mint,
        token_program_id,
        total_lamports,
        pool_token_supply,
        sol_withdraw_authority,
        sol_withdrawal_fee,
    })
}

// ─── Live per-LST state ────────────────────────────────────────────────────
#[derive(Debug)]
struct LstEntry {
    mint_id: AccountId,
    pool_state_id: AccountId,
    pool_state_pk: Pubkey,

    /// Zero (default `Pubkey`) until the first `pool_state` update arrives.
    reserve_stake_pk: Pubkey,
    /// Whether `reserve_stake_pk`'s subscription has already been queued
    /// (queue once, on first sight -- same convention as Sanctum/Orca).
    reserve_subscribed: bool,
    manager_fee_account: Pubkey,
    pool_mint: Pubkey,
    token_program_id: Pubkey,
    total_lamports: u64,
    pool_token_supply: u64,
    sol_withdraw_authority: Option<Pubkey>,
    /// (numerator, denominator)
    sol_withdrawal_fee: (u64, u64),

    /// Live lamport balance of `reserve_stake_pk`, from `header.lamports`
    /// (native account, not SPL -- no `on_token` needed).
    reserve_stake_lamports: u64,
}

// No `#[derive(Debug)]` -- `SubscriptionRequest` doesn't implement it (same
// reason `orca.rs`'s `OrcaState`, which has the same pending-subscription
// queue shape, doesn't derive Debug either).
pub struct SplStakePoolState {
    program_id: AccountId,
    lsts: Vec<LstEntry>,
    pool_state_to_idx: HashMap<AccountId, usize>,
    reserve_to_idx: HashMap<AccountId, usize>,
    /// Reserve-account subscriptions discovered mid-stream (once a
    /// `pool_state` is parsed) -- drained by `flush_pool`, same
    /// cascading-discovery pattern as `orca.rs`'s tick-array subscriptions.
    pending_sub_queue: SubscriptionQueue,
}

impl std::fmt::Debug for SplStakePoolState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SplStakePoolState").finish()
    }
}

impl SplStakePoolState {
    /// Builds this dex's live state and returns its pending subscription
    /// requests alongside it -- doesn't subscribe itself. See
    /// `dex::raydium::amm::RaydiumAmm::new`'s doc comment for why (paced
    /// through a shared [`crate::graph::SubscriptionQueue`] owned by
    /// `DexState` instead). The mid-stream reserve-account cascade
    /// (`l_pending_sub`/`q_hold_sub`/`flush_pool`) is unrelated and
    /// untouched -- it already has its own deferred-subscribe shape.
    pub fn new() -> (Self, Vec<SubscriptionRequest>) {
        // 20 "Spl"-kind LSTs as of this writing (see module doc) -- a
        // starting capacity, not a cap; grows fine if Sanctum's registry
        // picks up more.
        let mut lsts = Vec::with_capacity(24);
        let mut pool_state_to_idx = HashMap::with_capacity(24);
        let mut l_req = Vec::with_capacity(24);

        for lst in crate::sanctum_config::SANCTUM_LSTS.iter() {
            if Pubkey::new_from_array(lst.sol_value_calculator) != SPL_CALCULATOR_PROGRAM {
                continue;
            }
            if lst.pool_state == [0u8; 32] {
                continue;
            }
            let mint_pk = Pubkey::new_from_array(lst.mint);
            let pool_state_pk = Pubkey::new_from_array(lst.pool_state);
            let pool_state_id = account_id_from_pubkey(&pool_state_pk);
            let idx = lsts.len();
            pool_state_to_idx.insert(pool_state_id, idx);
            l_req.push(SubscriptionRequest { root: pool_state_id, filter_weight: 0, depth: 1 });
            lsts.push(LstEntry {
                mint_id: account_id_from_pubkey(&mint_pk),
                pool_state_id,
                pool_state_pk,
                reserve_stake_pk: Pubkey::default(),
                reserve_subscribed: false,
                manager_fee_account: Pubkey::default(),
                pool_mint: Pubkey::default(),
                token_program_id: Pubkey::default(),
                total_lamports: 0,
                pool_token_supply: 0,
                sol_withdraw_authority: None,
                sol_withdrawal_fee: (0, 0),
                reserve_stake_lamports: 0,
            });
        }

        let state = Self {
            program_id: account_id_from_pubkey(&STAKE_POOL_PROGRAM_ID),
            lsts,
            pool_state_to_idx,
            reserve_to_idx: HashMap::with_capacity(2),
            pending_sub_queue: SubscriptionQueue::default(),
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

    /// How many of the tracked LSTs have live pricing data -- for the
    /// periodic "pool stats" log.
    pub fn ready_count(&self) -> usize {
        self.lsts
            .iter()
            .filter(|l| l.total_lamports != 0 && l.pool_token_supply != 0 && l.reserve_stake_lamports != 0)
            .count()
    }

    /// Build a `WithdrawSolWithSlippage` instruction and append it to `wallet`.
    pub fn swap(
        &self,
        mint_id: AccountId,
        amount_in: u64,
        min_amount_out: u64,
        user_wallet: AccountId,
        user_pool_token_account: AccountId,
        user_sol_destination: AccountId,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        let lst = self
            .lsts
            .iter()
            .find(|l| l.mint_id == mint_id)
            .ok_or(TraderError::WrongMints)?;
        if lst.pool_mint == Pubkey::default() {
            return Err(TraderError::PoolNotReady);
        }
        let resolve = |id: AccountId| -> Result<Pubkey, TraderError> {
            pubkey_from_account_id(&id).ok_or(TraderError::PubkeyResolutionFailed(id))
        };
        let user_wallet_pk = resolve(user_wallet)?;
        let user_pool_token_pk = resolve(user_pool_token_account)?;
        let user_sol_pk = resolve(user_sol_destination)?;

        let (withdraw_authority, _bump) = Pubkey::find_program_address(
            &[lst.pool_state_pk.as_ref(), b"withdraw"],
            &STAKE_POOL_PROGRAM_ID,
        );

        let mut data = Vec::with_capacity(17);
        data.push(DISC_WITHDRAW_SOL_WITH_SLIPPAGE);
        data.extend_from_slice(&amount_in.to_le_bytes());
        data.extend_from_slice(&min_amount_out.to_le_bytes());

        let mut accounts = vec![
            AccountMeta::new(lst.pool_state_pk, false), // stake_pool
            AccountMeta::new_readonly(withdraw_authority, false), // withdraw_authority
            AccountMeta::new_readonly(user_wallet_pk, true), // user_transfer_authority (signer)
            AccountMeta::new(user_pool_token_pk, false), // burn_from_pool
            AccountMeta::new(lst.reserve_stake_pk, false), // reserve_stake
            AccountMeta::new(user_sol_pk, false),        // destination_lamports
            AccountMeta::new(lst.manager_fee_account, false), // manager_fee
            AccountMeta::new(lst.pool_mint, false),      // pool_mint
            AccountMeta::new_readonly(SYSVAR_CLOCK_ID, false),
            AccountMeta::new_readonly(SYSVAR_STAKE_HISTORY_ID, false),
            AccountMeta::new_readonly(STAKE_PROGRAM_ID, false),
            AccountMeta::new_readonly(lst.token_program_id, false),
        ];
        if let Some(sol_withdraw_authority) = lst.sol_withdraw_authority {
            accounts.push(AccountMeta::new_readonly(sol_withdraw_authority, true));
        }

        wallet.require_signer(user_wallet);
        wallet.append_ix(
            Instruction { program_id: STAKE_POOL_PROGRAM_ID, accounts, data },
            STAKE_POOL_WITHDRAW_SOL_CU,
        );
        Ok(())
    }

    /// Build the swap instruction for one `Hop` routed through this dex --
    /// same uniform adapter shape as every other dex module's `plan_hop`.
    /// One-directional (LST -> SOL only); `hop.input_mint` is the LST.
    pub fn plan_hop(
        &self,
        hop: &Hop,
        owner: AccountId,
        source_ata: AccountId,
        dest_ata: AccountId,
        wallet: &mut Wallet,
    ) -> Result<(), TraderError> {
        self.swap(hop.input_mint, hop.amount_in, hop.amount_out, owner, source_ata, dest_ata, wallet)
    }

    /// Bit-exact real on-chain `WithdrawSolWithSlippage` payout (this
    /// module's own doc comment has the formula:
    /// `fee_lamports = ceil(pool_tokens * fee_num / fee_denom)`,
    /// `lamports_out = pool_tokens_after_fee * total_lamports /
    /// pool_token_supply`) -- unlike [`Self::upsert_lst_edge`]'s linear
    /// `price`/`fee_frac` router-edge approximation (fine for route
    /// *discovery*, not exact enough for execution), this exists so
    /// `planner::reverify_hops` can re-quote this hop with the real
    /// integer math right before sending, the same discipline
    /// `OrcaState::exact_quote`/`RaydiumClmm::clmm_exact_quote` already
    /// apply to their own dex types (`planner.rs` previously only
    /// special-cased those two, on the documented assumption every other
    /// dex type's generic edge quote was "already exact" -- true for
    /// genuine constant-product pools, false for this one).
    ///
    /// Real, live-confirmed incident (2026-09-04): the generic linear
    /// edge quote overestimated a real jitoSOL withdrawal's payout by
    /// enough that the very next hop's swap failed on-chain with a real
    /// `insufficient funds` (SPL token transfers are exact-amount, no
    /// partial fill -- even a 1-lamport overestimate is enough),
    /// repeatedly, across 5+ real consecutive attempts trying to close
    /// the same stuck long leg.
    ///
    /// `None` if the pool is unknown, `input_mint` isn't this LST
    /// (one-directional, LST -> SOL only, matching [`Self::plan_hop`]'s
    /// own doc), the pool's real state isn't valid yet (same four
    /// conditions [`Self::upsert_lst_edge`] gates on), or the real payout
    /// would exceed the pool's real `reserve_stake_lamports` liquidity
    /// (the real instruction would fail the same way).
    pub fn exact_quote(&self, pool_id: AccountId, input_mint: AccountId, amount_in: u64) -> Option<u64> {
        let &idx = self.pool_state_to_idx.get(&pool_id)?;
        let lst = &self.lsts[idx];
        if lst.mint_id != input_mint {
            return None;
        }
        if lst.total_lamports == 0
            || lst.pool_token_supply == 0
            || lst.reserve_stake_lamports == 0
            || lst.sol_withdrawal_fee.1 == 0
        {
            return None;
        }
        let (fee_num, fee_denom) = lst.sol_withdrawal_fee;
        let pool_tokens = amount_in as u128;
        let fee_lamports = (pool_tokens * fee_num as u128).div_ceil(fee_denom as u128);
        let pool_tokens_after_fee = pool_tokens.saturating_sub(fee_lamports);
        let lamports_out = pool_tokens_after_fee * lst.total_lamports as u128 / lst.pool_token_supply as u128;
        let lamports_out = u64::try_from(lamports_out).ok()?;
        if lamports_out > lst.reserve_stake_lamports {
            return None;
        }
        Some(lamports_out)
    }

    /// Re-derive and upsert one LST's SOL-withdraw edge. `valid` gates
    /// `price`/`fee_frac`/`reserve_in` to `0.0`/`0.0`/`0` rather than
    /// skipping the `add_directed_edge` call on any of the four original
    /// invalidity conditions, so its own `price_out_per_in <= 0.0` gate
    /// reliably removes a stale edge left over from when this LST was
    /// last valid (computing `fee_frac` as a real `0/0` division would
    /// produce `NaN`, whose `Range::contains` is always `false` and so
    /// would happen to trigger the same removal path -- but relying on
    /// that would be fragile, hence the explicit `valid` gate instead).
    fn upsert_lst_edge(&self, idx: usize, router: &mut TradeRouter) {
        let lst = &self.lsts[idx];
        let wsol_id = account_id_from_pubkey(&WSOL_MINT);
        let valid = lst.total_lamports != 0
            && lst.pool_token_supply != 0
            && lst.reserve_stake_lamports != 0
            && lst.sol_withdrawal_fee.1 != 0;
        let price = if valid { lst.total_lamports as f64 / lst.pool_token_supply as f64 } else { 0.0 };
        let fee_frac = if valid {
            lst.sol_withdrawal_fee.0 as f64 / lst.sol_withdrawal_fee.1 as f64
        } else {
            0.0
        };
        // reserve_stake_lamports is the real constraining SOL-side
        // liquidity; reserve_in is a notional LST-equivalent (the
        // pool-token side isn't tracked live -- not needed for pricing,
        // only for the instruction) -- same simplification as
        // marinade.rs's batch_router.
        let reserve_out = lst.reserve_stake_lamports;
        let reserve_in = if valid { (reserve_out as f64 / price) as u64 } else { 0 };
        router.add_directed_edge(
            lst.pool_state_id,
            lst.mint_id,
            wsol_id,
            price,
            fee_frac,
            reserve_in,
            reserve_out,
            DexType::SplStakePoolWithdrawSol,
        );
    }
}

impl Updater for SplStakePoolState {
    fn on_account(&mut self, header: &Header, body: &[u8]) {
        if let Some(&idx) = self.pool_state_to_idx.get(&header.accountid) {
            let Some(parsed) = parse_stake_pool(body) else { return };
            self.lsts[idx].manager_fee_account = parsed.manager_fee_account;
            self.lsts[idx].pool_mint = parsed.pool_mint;
            self.lsts[idx].token_program_id = parsed.token_program_id;
            self.lsts[idx].total_lamports = parsed.total_lamports;
            self.lsts[idx].pool_token_supply = parsed.pool_token_supply;
            self.lsts[idx].sol_withdraw_authority = parsed.sol_withdraw_authority;
            self.lsts[idx].sol_withdrawal_fee = parsed.sol_withdrawal_fee;
            if !self.lsts[idx].reserve_subscribed {
                self.lsts[idx].reserve_stake_pk = parsed.reserve_stake;
                let reserve_id = account_id_from_pubkey(&parsed.reserve_stake);
                self.reserve_to_idx.insert(reserve_id, idx);
                self.pending_sub_queue.push(SubscriptionRequest {
                    root: reserve_id,
                    filter_weight: 0,
                    depth: 1,
                });
                self.lsts[idx].reserve_subscribed = true;
            }
            return;
        }
        if let Some(&idx) = self.reserve_to_idx.get(&header.accountid) {
            self.lsts[idx].reserve_stake_lamports = header.lamports;
        }
    }

    fn on_token(&mut self, _ta: &Tokenaccountv1) -> bool {
        false
    }

    fn on_tx(&mut self, _ix: &CatscopeInstructionRead<'_>, _slot: &solana_sdk::clock::Slot) {}

    /// Add one LST → SOL withdraw-sol edge per LST with known live pricing.
    fn batch_router(&mut self, router: &mut TradeRouter) {
        for idx in 0..self.lsts.len() {
            self.upsert_lst_edge(idx, router);
        }
    }

    /// `pool_state_to_idx` and `reserve_to_idx` both key by an account
    /// that only ever affects one LST's own edge -- unlike Sanctum,
    /// there's no global-fee-style account here that would require
    /// re-deriving every LST at once.
    fn refresh_account_router(&mut self, account_id: AccountId, router: &mut TradeRouter) {
        if let Some(&idx) = self.pool_state_to_idx.get(&account_id) {
            self.upsert_lst_edge(idx, router);
        } else if let Some(&idx) = self.reserve_to_idx.get(&account_id) {
            self.upsert_lst_edge(idx, router);
        }
    }

    // Paced through `pending_sub_queue` -- see `orca.rs::flush_pool`'s
    // doc comment for why an unbounded per-commit batch here is exactly
    // the bug that produced this session's real `stdio timeout` hangs.
    fn flush_pool(&mut self, g: &Graph, max_per_flush: usize) -> Result<(), CatscopeGuestError> {
        self.pending_sub_queue.flush(g, max_per_flush)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal, synthetic `StakePool` byte buffer with every
    /// `Option`/`FutureEpoch` field `Some`-valued (real JitoSOL/bSOL pools
    /// are all `None` today, so that path is otherwise untested).
    fn synthetic_stake_pool() -> Vec<u8> {
        let mut b = Vec::new();
        b.push(1u8); // account_type
        b.extend_from_slice(&[1u8; 32]); // manager
        b.extend_from_slice(&[2u8; 32]); // staker
        b.extend_from_slice(&[3u8; 32]); // stake_deposit_authority
        b.push(255); // stake_withdraw_bump_seed
        b.extend_from_slice(&[4u8; 32]); // validator_list
        b.extend_from_slice(&[5u8; 32]); // reserve_stake
        b.extend_from_slice(&[6u8; 32]); // pool_mint
        b.extend_from_slice(&[7u8; 32]); // manager_fee_account
        b.extend_from_slice(&[8u8; 32]); // token_program_id
        b.extend_from_slice(&1_000_000u64.to_le_bytes()); // total_lamports
        b.extend_from_slice(&900_000u64.to_le_bytes()); // pool_token_supply
        b.extend_from_slice(&500u64.to_le_bytes()); // last_update_epoch
        b.extend_from_slice(&0i64.to_le_bytes()); // lockup.unix_timestamp
        b.extend_from_slice(&0u64.to_le_bytes()); // lockup.epoch
        b.extend_from_slice(&[0u8; 32]); // lockup.custodian
        b.extend_from_slice(&100u64.to_le_bytes()); // epoch_fee.denominator
        b.extend_from_slice(&1u64.to_le_bytes()); // epoch_fee.numerator
        b.push(1); // next_epoch_fee: Some
        b.extend_from_slice(&100u64.to_le_bytes());
        b.extend_from_slice(&2u64.to_le_bytes());
        b.push(1); // preferred_deposit: Some
        b.extend_from_slice(&[9u8; 32]);
        b.push(1); // preferred_withdraw: Some
        b.extend_from_slice(&[10u8; 32]);
        b.extend_from_slice(&0u64.to_le_bytes()); // stake_deposit_fee.denominator
        b.extend_from_slice(&0u64.to_le_bytes()); // stake_deposit_fee.numerator
        b.extend_from_slice(&1000u64.to_le_bytes()); // stake_withdrawal_fee.denominator
        b.extend_from_slice(&1u64.to_le_bytes()); // stake_withdrawal_fee.numerator
        b.push(2); // next_stake_withdrawal_fee: Two(Fee)
        b.extend_from_slice(&1000u64.to_le_bytes());
        b.extend_from_slice(&1u64.to_le_bytes());
        b.push(0); // stake_referral_fee
        b.push(1); // sol_deposit_authority: Some
        b.extend_from_slice(&[11u8; 32]);
        b.extend_from_slice(&0u64.to_le_bytes()); // sol_deposit_fee.denominator
        b.extend_from_slice(&0u64.to_le_bytes()); // sol_deposit_fee.numerator
        b.push(0); // sol_referral_fee
        b.push(1); // sol_withdraw_authority: Some
        b.extend_from_slice(&[12u8; 32]);
        b.extend_from_slice(&10_000u64.to_le_bytes()); // sol_withdrawal_fee.denominator
        b.extend_from_slice(&10u64.to_le_bytes()); // sol_withdrawal_fee.numerator
        b
    }

    #[test]
    fn parses_synthetic_pool_with_all_options_present() {
        let data = synthetic_stake_pool();
        let parsed = parse_stake_pool(&data).expect("should parse");
        assert_eq!(parsed.reserve_stake, Pubkey::new_from_array([5u8; 32]));
        assert_eq!(parsed.pool_mint, Pubkey::new_from_array([6u8; 32]));
        assert_eq!(parsed.manager_fee_account, Pubkey::new_from_array([7u8; 32]));
        assert_eq!(parsed.token_program_id, Pubkey::new_from_array([8u8; 32]));
        assert_eq!(parsed.total_lamports, 1_000_000);
        assert_eq!(parsed.pool_token_supply, 900_000);
        assert_eq!(parsed.sol_withdraw_authority, Some(Pubkey::new_from_array([12u8; 32])));
        assert_eq!(parsed.sol_withdrawal_fee, (10, 10_000));
    }

    #[test]
    fn parses_real_jitosol_and_bsol_shape_with_all_options_absent() {
        // Same field layout, but every Option/FutureEpoch is None -- the
        // shape both real, live-verified JitoSOL/bSOL pools actually have.
        let mut b = Vec::new();
        b.push(1u8);
        b.extend_from_slice(&[0u8; 32 * 3]); // manager, staker, stake_deposit_authority
        b.push(253);
        b.extend_from_slice(&[0u8; 32]); // validator_list
        b.extend_from_slice(&[9u8; 32]); // reserve_stake
        b.extend_from_slice(&[1u8; 32]); // pool_mint
        b.extend_from_slice(&[0u8; 32 * 2]); // manager_fee_account, token_program_id
        b.extend_from_slice(&0u64.to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes());
        b.extend_from_slice(&[0u8; 48]); // lockup
        b.extend_from_slice(&0u64.to_le_bytes()); // epoch_fee
        b.extend_from_slice(&0u64.to_le_bytes());
        b.push(0); // next_epoch_fee: None
        b.push(0); // preferred_deposit: None
        b.push(0); // preferred_withdraw: None
        b.extend_from_slice(&0u64.to_le_bytes()); // stake_deposit_fee
        b.extend_from_slice(&0u64.to_le_bytes());
        b.extend_from_slice(&1000u64.to_le_bytes()); // stake_withdrawal_fee
        b.extend_from_slice(&1u64.to_le_bytes());
        b.push(0); // next_stake_withdrawal_fee: None
        b.push(0); // stake_referral_fee
        b.push(0); // sol_deposit_authority: None
        b.extend_from_slice(&0u64.to_le_bytes()); // sol_deposit_fee
        b.extend_from_slice(&0u64.to_le_bytes());
        b.push(0); // sol_referral_fee
        b.push(0); // sol_withdraw_authority: None
        b.extend_from_slice(&1000u64.to_le_bytes()); // sol_withdrawal_fee
        b.extend_from_slice(&1u64.to_le_bytes());

        let parsed = parse_stake_pool(&b).expect("should parse");
        assert_eq!(parsed.sol_withdraw_authority, None);
        assert_eq!(parsed.sol_withdrawal_fee, (1, 1000));
    }

    #[test]
    fn truncated_data_returns_none() {
        assert!(parse_stake_pool(&[0u8; 10]).is_none());
    }

    // --- exact_quote -----------------------------------------------------

    fn synthetic_lst_entry(mint_id: AccountId, pool_state_id: AccountId) -> LstEntry {
        LstEntry {
            mint_id,
            pool_state_id,
            pool_state_pk: Pubkey::new_unique(),
            reserve_stake_pk: Pubkey::new_unique(),
            reserve_subscribed: true,
            manager_fee_account: Pubkey::default(),
            pool_mint: Pubkey::default(),
            token_program_id: Pubkey::default(),
            total_lamports: 1_000_000,
            pool_token_supply: 900_000,
            sol_withdraw_authority: None,
            sol_withdrawal_fee: (1, 1000), // 0.1%
            reserve_stake_lamports: 10_000_000,
        }
    }

    fn state_with_lst(lst: LstEntry) -> SplStakePoolState {
        let mut pool_state_to_idx = HashMap::new();
        pool_state_to_idx.insert(lst.pool_state_id, 0);
        SplStakePoolState {
            program_id: 1,
            lsts: vec![lst],
            pool_state_to_idx,
            reserve_to_idx: HashMap::new(),
            pending_sub_queue: SubscriptionQueue::default(),
        }
    }

    #[test]
    fn exact_quote_matches_real_ceil_then_floor_formula() {
        // total_lamports=1_000_000, pool_token_supply=900_000, fee=1/1000.
        // amount_in=100_000: fee_lamports = ceil(100_000/1000) = 100
        // (exact); pool_tokens_after_fee = 99_900;
        // lamports_out = 99_900 * 1_000_000 / 900_000 = 111_000.
        let state = state_with_lst(synthetic_lst_entry(7, 42));
        assert_eq!(state.exact_quote(42, 7, 100_000), Some(111_000));
    }

    #[test]
    fn exact_quote_rounds_the_fee_up_not_down() {
        // fee_lamports = ceil(333 * 1 / 1000) = ceil(0.333) = 1, not 0 --
        // real on-chain behavior per this module's own doc comment.
        // pool_tokens_after_fee = 332;
        // lamports_out = 332 * 1_000_000 / 900_000 = 368 (floor).
        let state = state_with_lst(synthetic_lst_entry(7, 42));
        assert_eq!(state.exact_quote(42, 7, 333), Some(368));
    }

    #[test]
    fn exact_quote_none_for_unknown_pool() {
        let state = state_with_lst(synthetic_lst_entry(7, 42));
        assert_eq!(state.exact_quote(999, 7, 100_000), None);
    }

    #[test]
    fn exact_quote_none_for_wrong_input_mint() {
        // One-directional (LST -> SOL only) -- a mismatched input_mint
        // must refuse, not silently quote against the wrong LST.
        let state = state_with_lst(synthetic_lst_entry(7, 42));
        assert_eq!(state.exact_quote(42, 8, 100_000), None);
    }

    #[test]
    fn exact_quote_none_when_pool_state_not_valid_yet() {
        let mut lst = synthetic_lst_entry(7, 42);
        lst.total_lamports = 0; // not yet updated from a real account
        let state = state_with_lst(lst);
        assert_eq!(state.exact_quote(42, 7, 100_000), None);
    }

    #[test]
    fn exact_quote_none_when_payout_exceeds_real_reserve_liquidity() {
        let mut lst = synthetic_lst_entry(7, 42);
        lst.reserve_stake_lamports = 50; // real payout (111_000) exceeds this
        let state = state_with_lst(lst);
        assert_eq!(state.exact_quote(42, 7, 100_000), None);
    }
}
