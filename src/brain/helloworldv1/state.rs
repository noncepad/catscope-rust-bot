use std::{
    cell::UnsafeCell,
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
    time::{Duration, Instant, SystemTime},
};

use solana_sdk::{
    clock::Slot,
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer,
};

use crate::{
    brain::helloworldv1::{
        message::{CustomMessageInbound, CustomMessageOutbound},
        Configuration,
    },
    catscope::witbot::{
        shooter::{Header, Tokenaccountv1},
        transactionprocessor,
    },
    err::CatscopeGuestError,
    event::{AccountWrapper, SlotStatus},
    graph::{AccountId, CommitHook, Graph, LowLatencyAccountUpdate},
    log_debug, log_error, log_info, log_warn,
    message::{InboundMesasgeHandler, MessageAction, MessageSend},
    trader::{dex::orca::OrcaState, types::SwapParams},
    txview::TransactionList,
    util::{account_id_from_pubkey, as_bytes_mut, pubkey_from_account_id, rc_unlock},
    wallet::Wallet,
    TradingSetup,
};

/// Minimum wSOL input to avoid LiquidityUnderflow (0.01 SOL, 9 decimals).
const MIN_SWAP_SOL: u64 = 10_000_000;
/// Minimum USDC input to avoid LiquidityUnderflow (0.10 USDC, 6 decimals).
const MIN_SWAP_USDC: u64 = 100_000;
/// Rent-exempt minimum for an SPL token account (165 bytes). Kept in wSOL ATA to hold it open.
const TOKEN_ACCOUNT_RENT: u64 = 2_039_280;

#[derive(Debug)]
enum Direction {
    ToSol,
    ToUSD,
}
#[derive(Debug)]
pub(crate) struct State {
    tx_count: usize,
    read_tx_count: usize,
    slot_delta_since_start: Slot,
    direction: Direction,
    last_slot: Slot,
    last_print: Instant,
    o_rc_keypair: Option<KeypairExtra>,
    o_bounce: Option<BounceStatus>,
    o_orca: Option<OrcaState>,
    m_sig: HashMap<Signature, (Slot, Instant)>,
    tx_latency: TxLatencyStats,
    q_sig: VecDeque<SlotSignatureSet>,
    tracker: TransactionTracker,
    pub(crate) echo: bool,
    recycle_q: VecDeque<SlotSignatureSet>,
    o_ata_sol: Option<AccountId>,
    o_ata_usd: Option<AccountId>,
}

/// make sure we do not send duplicate transactions
#[derive(Debug, Default)]
struct TransactionTracker {
    // swap from SOL to USD
    o_tx_sol_to_usd: Option<Slot>,
}

#[derive(Debug, Default)]
struct TxLatencyStats {
    samples: Vec<u64>,
}

impl TxLatencyStats {
    fn record(&mut self, elapsed: Duration) {
        self.samples.push(elapsed.as_micros() as u64);
    }
    fn percentile(&mut self, p: usize) -> Option<u64> {
        if self.samples.is_empty() {
            return None;
        }
        self.samples.sort_unstable();
        let idx = (self.samples.len().saturating_sub(1) * p) / 100;
        Some(self.samples[idx])
    }
    fn count(&self) -> usize {
        self.samples.len()
    }
}

#[derive(Debug)]
struct SignatureWithSlot {
    sent_slot: Slot,
    signature: Signature,
}

#[derive(Debug, Default)]
struct SlotSignatureSet {
    slot: Slot,
    hs_sig: HashSet<Signature>,
}
#[derive(Debug)]
struct BounceStatus {
    last_slot: Slot,
}

#[derive(Debug)]
struct KeypairExtra {
    rc_keypair: Rc<UnsafeCell<Keypair>>,
    account_id: AccountId,
}
impl State {
    fn wallet(&self) -> Option<AccountId> {
        let ke = self.o_rc_keypair.as_ref()?;
        Some(ke.account_id)
    }
}
impl Default for State {
    fn default() -> Self {
        Self {
            slot_delta_since_start: 0,
            tx_count: 0,
            read_tx_count: 0,
            direction: Direction::ToSol,
            o_orca: None,
            o_bounce: None,
            last_slot: 0,
            last_print: Instant::now(),
            o_rc_keypair: None,
            m_sig: HashMap::default(),
            tx_latency: TxLatencyStats::default(),
            q_sig: VecDeque::default(),
            tracker: TransactionTracker::default(),
            recycle_q: VecDeque::default(),
            echo: false,
            o_ata_sol: None,
            o_ata_usd: None,
        }
    }
}

pub(crate) struct StateHelper<'a> {
    pub(crate) graph: &'a mut Graph,
    pub(crate) nonce: &'a mut u32,
    pub(crate) o_commit_slot: Option<Slot>,
    pub(crate) state: &'a mut State,
    pub(crate) wallet: &'a mut Wallet,
    pub(crate) configuration: &'a mut Configuration,
    pub(crate) q_msg: &'a mut VecDeque<MessageSend<CustomMessageOutbound>>,
}

impl<'a> StateHelper<'a> {
    pub(crate) fn nonce_check(&mut self, other_nonce: u32) -> Result<(), CatscopeGuestError> {
        if *self.nonce != other_nonce {
            return Err(CatscopeGuestError::BadNonce(*self.nonce, other_nonce));
        }
        *self.nonce += 1;
        Ok(())
    }
    pub(crate) fn on_load(&mut self, trading: &TradingSetup) {
        self.configuration.count += 1;
        assert_eq!(self.configuration.count, 1);
        self.state.echo = std::env::args().any(|a| a == "--echo");

        self.state
            .o_orca
            .replace(OrcaState::new(self.graph, trading).unwrap());
        if let Some(x) = self.state.o_rc_keypair.as_ref() {
            self.configuration.set(&x.rc_keypair);
        }
        log_info!("bot has been successfully uploaded to validator");
    }
    pub(crate) fn on_slot_status(&mut self, slot: Slot, status: SlotStatus) {
        match status {
            SlotStatus::Processed => {}
            SlotStatus::Rooted => {}
            SlotStatus::Confirmed => {}
            SlotStatus::FirstShredReceived => {}
            SlotStatus::Completed => {}
            SlotStatus::CreatedBank => {}
            SlotStatus::Dead => log_info!("slot {slot}; status dead"),
        }
    }

    pub(crate) fn low_latency(&mut self, mut llap: LowLatencyAccountUpdate) {
        let mut orca = self.state.o_orca.take().unwrap();
        let do_stuff = *self.nonce == 0;
        if do_stuff {
            log_warn!(
                "low_latency hit!!!!!!!!!!!!!!! {} {}",
                llap.token_len(),
                llap.account_len()
            );
        }
        *self.nonce += 1;
        while let Some(ta) = llap.token() {
            {
                let db = self.wallet.token_mut();
                db.on_token(ta, false);
            }
            orca.on_token(ta).unwrap();
        }
        let zero = [];
        while let Some(account) = llap.account() {
            let d = if let Some(x) = account.body { x } else { &zero };

            orca.on_account(account.header, d, false).unwrap();
        }
        self.state.o_orca.replace(orca);
    }

    pub(crate) fn mid_on_tx(&mut self, mut transaction_list: TransactionList) {
        let mut l_a_size;
        let mut l_account_id = [0u64; 256];
        let mut signature;
        let mut l_program_id = [0u64; 256];
        let mut prog_i;
        while let Some((mut tx, result)) = transaction_list.transaction() {
            if result.is_err() {
                continue;
            }
            let slot = result.unwrap();
            l_a_size = tx.account.len();
            let inner_n = tx.ix_inner_len();
            let outer_n = tx.ix_outer_len();
            let l_a_subbuf = &mut l_account_id[0..l_a_size];
            l_a_subbuf.copy_from_slice(tx.account);
            l_a_subbuf.sort_unstable();
            {
                prog_i = 0;
                // track inner instructions
                for k in 0..inner_n {
                    let ix = tx.ix_inner(k);
                    l_program_id[prog_i] = *ix.program();
                    prog_i += 1;
                    tx = ix.into();
                }
                let l_prog = {
                    let subbuf = &mut l_program_id[0..prog_i];
                    subbuf.sort_unstable();
                    &l_program_id[0..prog_i]
                };
                // feed program_id index to various smart contract handlers
                if let Some(orca) = self.state.o_orca.as_mut() {
                    let p_id = *orca.program_id();
                    if let Ok(i) = l_prog.binary_search(&p_id) {
                        'doneix1: for k in i..inner_n {
                            let ix = tx.ix_inner(k);
                            if *ix.program() != p_id {
                                tx = ix.into();
                                break 'doneix1;
                            }
                            orca.on_tx(&ix, &slot);
                            tx = ix.into();
                        }
                    }
                }
            }
            {
                prog_i = 0;
                // track outer instructions
                for k in 0..outer_n {
                    let ix = tx.ix_outer(k);
                    l_program_id[prog_i] = *ix.program();
                    prog_i += 1;
                    tx = ix.into();
                }
                let l_prog = {
                    let subbuf = &mut l_program_id[0..prog_i];
                    subbuf.sort_unstable();
                    &l_program_id[0..prog_i]
                };
                // feed program_id index to various smart contract handlers
                if let Some(orca) = self.state.o_orca.as_mut() {
                    let p_id = *orca.program_id();
                    if let Ok(i) = l_prog.binary_search(&p_id) {
                        'doneix1: for k in i..outer_n {
                            let ix = tx.ix_outer(k);
                            if *ix.program() != p_id {
                                tx = ix.into();
                                break 'doneix1;
                            }
                            orca.on_tx(&ix, &slot);
                            tx = ix.into();
                        }
                    }
                }
            }
            signature = Signature::from(*tx.signature);
            self.state.read_tx_count += 1;
            if self.state.read_tx_count % 50_000 == 0 {
                log_warn!("read_tx_count {}", self.state.read_tx_count);
            }
            if let Some((slot2, sent_at)) = self.state.m_sig.remove(&signature) {
                let elapsed = sent_at.elapsed();
                self.state.tx_latency.record(elapsed);
                log_warn!(
                    "got transaction result {} {} {:?}; latency {}µs",
                    slot2,
                    signature,
                    slot2,
                    elapsed.as_micros()
                );
            }
        }
    }

    pub(crate) fn evaluate(&mut self) {
        if self.configuration.wallet == 0 {
            if let Some(x) = self.state.o_rc_keypair.as_ref() {
                self.configuration.set(&x.rc_keypair);
                let db = self.wallet.token_mut();
                let owner = self.state.wallet().unwrap();
                let l_a = db.balance(&owner, &self.configuration.mint_sol, true);
                if !l_a.is_empty() {
                    panic!("got token update {l_a:?}")
                }
            } else {
                return;
            }
        }
        if self.state.slot_delta_since_start < 20 {
            return;
        }
        if 10 < self.state.tx_count {
            return;
        }

        let has_check_pool;
        let (orca_count, parsed_orca_count, tx_orca_count);
        if let Some(orca) = self.state.o_orca.as_ref() {
            (orca_count, parsed_orca_count, tx_orca_count) = orca.count();
            has_check_pool = orca.has_check_pool();
        } else {
            orca_count = 0;
            parsed_orca_count = 0;
            tx_orca_count = 0;
            has_check_pool = false;
        }
        if self.state.last_print.elapsed() >= Duration::from_secs(20) {
            let owner = self.configuration.wallet;
            let db = self.wallet.token_mut();
            let (bal_sol, _) = db
                .balance(&owner, &self.configuration.mint_sol, true)
                .first()
                .map(|(a, b)| (*a, *b))
                .unwrap_or_default();
            let (bal_usd, _) = db
                .balance(&owner, &self.configuration.mint_usdc, true)
                .first()
                .map(|(a, b)| (*a, *b))
                .unwrap_or_default();
            let p50 = self.state.tx_latency.percentile(50);
            let p99 = self.state.tx_latency.percentile(99);
            let n = self.state.tx_latency.count() as u64;
            log_info!(
                "slot {}; orca count {} {} {}; has check pool {}; balance {} {}; tx latency n={} p50={:?}µs p99={:?}µs",
                self.state.last_slot,
                orca_count,
                parsed_orca_count,
                tx_orca_count,
                has_check_pool,
                bal_sol,
                bal_usd,
                n,
                p50,
                p99,
            );
            self.q_msg.push_back(MessageSend::Custom(
                CustomMessageOutbound::TxLatencyReport {
                    n,
                    p50_us: p50.unwrap_or(0),
                    p99_us: p99.unwrap_or(0),
                },
            ));
            self.state.last_print = Instant::now();
        }
        let db = self.wallet.token_mut();
        let o_sol = db
            .balance(
                &self.configuration.wallet,
                &self.configuration.mint_sol,
                false,
            )
            .first()
            .map(|(x1, x2)| (*x1, *x2));

        if self.state.o_ata_sol.is_none() {
            if let Some((a_id, _)) = o_sol {
                self.state.o_ata_sol = Some(a_id);
            }
        }
        let needs_create_ata_sol = o_sol.is_none() && self.state.o_ata_sol.is_none();

        let o_usd = db
            .balance(
                &self.configuration.wallet,
                &self.configuration.mint_usdc,
                false,
            )
            .first()
            .map(|(x1, x2)| (*x1, *x2));
        {
            let t = Instant::now();
            let x = t.duration_since(self.configuration.start);
            if Duration::from_secs(120) < x {
                panic!("time expired {x:?}; {o_sol:?} {o_usd:?}")
            }
        }
        if self.state.o_ata_usd.is_none() {
            if let Some((a_id, _)) = o_usd {
                self.state.o_ata_usd = Some(a_id);
            }
        }
        let needs_create_ata_usd = o_usd.is_none() && self.state.o_ata_usd.is_none();
        // Don't send another swap while one is already in flight.
        if !self.state.m_sig.is_empty() {
            return;
        }
        let o_params = match self.state.direction {
            Direction::ToSol => {
                if let Some((token_account_id, amount)) = o_usd {
                    self.state.o_ata_usd = Some(token_account_id);
                    if amount < MIN_SWAP_USDC {
                        log_debug!(
                            "USDC balance {} below minimum {}; skipping swap",
                            amount,
                            MIN_SWAP_USDC
                        );
                        return;
                    }
                    let mut sp = SwapParams {
                        pool: 0,
                        input_mint: self.configuration.mint_usdc,
                        output_mint: self.configuration.mint_sol,
                        amount_in: amount,
                        min_amount_out: 1,
                        user_source_token_account: token_account_id,
                        user_destination_token_account: 0,
                        user_wallet: self.configuration.wallet,
                    };
                    if let Some((other_token_account_id, _)) = o_sol {
                        self.state.o_ata_sol = Some(other_token_account_id);
                        sp.user_destination_token_account = other_token_account_id;
                    } else if let Some(cached) = self.state.o_ata_sol {
                        sp.user_destination_token_account = cached;
                    }
                    Some(sp)
                } else {
                    None
                }
            }
            Direction::ToUSD => {
                if let Some((token_account_id, amount)) = o_sol {
                    self.state.o_ata_sol = Some(token_account_id);
                    let amount_in = amount.saturating_sub(TOKEN_ACCOUNT_RENT);
                    if amount_in < MIN_SWAP_SOL {
                        log_warn!(
                            "wSOL balance {} (spendable {}) below minimum {}; skipping swap",
                            amount,
                            amount_in,
                            MIN_SWAP_SOL
                        );
                        return;
                    }
                    let mut sp = SwapParams {
                        pool: 0,
                        input_mint: self.configuration.mint_sol,
                        output_mint: self.configuration.mint_usdc,
                        amount_in,
                        min_amount_out: 1,
                        user_source_token_account: token_account_id,
                        user_destination_token_account: 0,
                        user_wallet: self.configuration.wallet,
                    };
                    if let Some((other_token_account_id, _)) = o_usd {
                        self.state.o_ata_usd = Some(other_token_account_id);
                        sp.user_destination_token_account = other_token_account_id;
                    } else if let Some(cached) = self.state.o_ata_usd {
                        sp.user_destination_token_account = cached;
                    }
                    Some(sp)
                } else {
                    None
                }
            }
        };
        if let Some(mut params) = o_params {
            if needs_create_ata_usd {
                // this is only true for a blank destination.
                // we would not be here if the source token account were missing
                if let Some(ata_id) = self
                    .wallet
                    .append_create_ata(self.configuration.wallet, self.configuration.mint_usdc)
                {
                    if params.user_destination_token_account == 0 {
                        params.user_destination_token_account = ata_id;
                        assert_ne!(params.output_mint, self.configuration.mint_sol);
                    }
                }
            }
            if needs_create_ata_sol {
                // this is only true for a blank destination.
                // we would not be here if the source token account were missing
                if let Some(ata_id) = self
                    .wallet
                    .append_create_ata(self.configuration.wallet, self.configuration.mint_sol)
                {
                    if params.user_destination_token_account == 0 {
                        params.user_destination_token_account = ata_id;
                        assert_ne!(params.output_mint, self.configuration.mint_usdc);
                    }
                }
            }
            if params.user_destination_token_account != 0 {
                if let Some(orca) = self.state.o_orca.as_ref() {
                    orca.swap(&mut params, self.wallet, self.configuration.max_slippage)
                        .expect("do token swap");
                }
            } else {
                log_warn!(
                    "destination token account unknown for mint {}; skipping swap",
                    params.output_mint
                );
            }
        }
        while let Some((sig, data)) = self.wallet.assemble() {
            match transactionprocessor::send(sig.as_array(), data) {
                Ok(_) => {
                    self.state.tx_count += 1;
                    let mut ss = if self
                        .state
                        .q_sig
                        .front()
                        .map_or(false, |s| s.slot == self.state.last_slot)
                    {
                        self.state.q_sig.pop_front().unwrap()
                    } else {
                        let mut ss2 = self.state.recycle_q.pop_front().unwrap_or_default();
                        ss2.hs_sig.clear();
                        ss2.slot = self.state.last_slot;
                        ss2
                    };
                    ss.hs_sig.insert(sig);
                    self.state
                        .m_sig
                        .insert(sig, (self.state.last_slot, Instant::now()));
                    self.state.q_sig.push_front(ss);
                    self.state.direction = match self.state.direction {
                        Direction::ToUSD => Direction::ToSol,
                        Direction::ToSol => Direction::ToUSD,
                    };
                }
                Err(e) => log_error!("failed to send tx {e}"),
            };
        }
    }
}

impl<'a> InboundMesasgeHandler<Configuration, CustomMessageInbound, CustomMessageOutbound>
    for StateHelper<'a>
{
    fn on_message(&mut self, action: MessageAction<Configuration, CustomMessageInbound>) {
        match action {
            MessageAction::Ping(_) => {
                self.q_msg.push_back(MessageSend::Pong(SystemTime::now()));
            }
            MessageAction::AdjustConfiguration(new_configuration) => {
                log_warn!("StateHelper::on_message------------------------------- adjust config");
                unsafe { std::ptr::copy_nonoverlapping(&new_configuration, self.configuration, 1) };
            }
            MessageAction::Shutdown => panic!("shutting down"),
            MessageAction::Custom(x) => {
                log_warn!("StateHelper::on_message------------------------------- custom");
                match x {
                    CustomMessageInbound::Blank => {
                        log_warn!("StateHelper::on_message------------------------------- blank");
                    }
                    CustomMessageInbound::EchoRequest(s) => {
                        log_warn!("StateHelper::on_message-------------------------------echo request {s}");
                        self.q_msg.push_back(MessageSend::Custom(
                            CustomMessageOutbound::EchoResponse(s.clone()),
                        ));
                    }
                    CustomMessageInbound::Wallet(rc_keypair) => {
                        let keypair = rc_unlock(&rc_keypair);
                        let pubkey = keypair.pubkey();
                        let account_id = account_id_from_pubkey(&pubkey);
                        self.state.o_ata_sol = None;
                        self.state.o_ata_usd = None;
                        log_warn!(
                            "StateHelper::on_message-------------------------------got keypair {} {}",
                            pubkey,account_id
                        );
                        self.wallet
                            .append_key(rc_keypair.clone(), self.graph)
                            .unwrap();
                        self.wallet.set_payer(account_id);
                        if let Some(x2) = self.state.o_rc_keypair.replace(KeypairExtra {
                            rc_keypair,
                            account_id,
                        }) {
                            let old_keypair = rc_unlock(&x2.rc_keypair);
                            log_warn!(
                                "StateHelper::on_message-------------------------------deleting keypair {} {}",
                                old_keypair.pubkey(),x2.account_id
                            );
                        }
                    }
                }
            }
        }
    }

    fn message_send(&mut self, message: MessageSend<CustomMessageOutbound>) {
        self.q_msg.push_back(message);
    }
}

impl<'a> CommitHook for StateHelper<'a> {
    fn start(&mut self, slot: Slot) {
        assert!(self.o_commit_slot.replace(slot).is_none());
        let (_, orca_ephemeral_count, orca_tx_count) =
            if let Some(orca) = self.state.o_orca.as_ref() {
                orca.count()
            } else {
                (0, 0, 0)
            };
        if slot % 100 == 0 {
            log_warn!(
                "CommitHook - start {slot}; orca {} {}; read_tx_count {}",
                orca_ephemeral_count,
                orca_tx_count,
                self.state.read_tx_count,
            );
        } else {
            log_warn!(
                "CommitHook - start {}; slot_delta_since_start {}; orca {} {}; read_tx_count {}",
                slot,
                self.state.slot_delta_since_start,
                orca_ephemeral_count,
                orca_tx_count,
                self.state.read_tx_count,
            );
        }
        self.state.last_slot = slot;
        'done1: while let Some(mut ss) = self.state.q_sig.pop_front() {
            if slot < ss.slot {
                self.state.q_sig.push_front(ss);
                break 'done1;
            }
            for sig in ss.hs_sig.drain() {
                if let Some((old_slot, _)) = self.state.m_sig.remove(&sig) {
                    log_warn!(
                        "transaction expired: slot {} -> {}; signature {}",
                        old_slot,
                        slot,
                        sig
                    );
                }
            }
            ss.slot = 0;
            assert!(ss.hs_sig.is_empty());
            self.state.recycle_q.push_back(ss);
        }
    }

    fn on_account(
        &mut self,
        header: &Header,
        body: &[u8],
        _o_m_from: Option<&HashSet<AccountId>>,
        _o_m_to: Option<&HashSet<AccountId>>,
    ) {
        let mut orca = self.state.o_orca.take().unwrap();
        orca.on_account(header, body, true).unwrap();
        self.state.o_orca.replace(orca);
    }

    fn on_token(&mut self, token_account: &Tokenaccountv1) {
        let db = self.wallet.token_mut();
        db.on_token(token_account, true);
    }

    fn finish(&mut self) {
        self.o_commit_slot = None;
        self.state.slot_delta_since_start += 1;
        if let Some(mut orca) = self.state.o_orca.take() {
            orca.flush_pool(self.graph).expect("flush pool");
            self.state.o_orca.replace(orca);
        }
    }
}
