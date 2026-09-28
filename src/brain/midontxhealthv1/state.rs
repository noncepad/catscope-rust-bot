//! Passive, listen-only completeness check for the Transaction-lane event
//! stream (`Event::Transaction` -> `mid_on_tx`) -- see this module's own
//! top-level (`mod.rs`) doc comment for the real, live-measured finding
//! that motivated it.
//!
//! The question this isolates: for a given slot, does `mid_on_tx` see
//! *every* non-vote transaction that really landed in that block, or does
//! it silently drop some?
//!
//! **This module's first version got this wrong**, and it's worth
//! recording why: it tracked the *largest* real Agave `tx.index` seen per
//! slot and compared `max + 1` against the count of distinct indices
//! seen, on the assumption `tx.index` was scoped to non-vote transactions
//! only. Real, user-caught, definitively confirmed 2026-09-09 (by logging
//! a sampled signature+index+slot and looking that exact signature up in
//! `solana block <slot>`'s own ordering): `tx.index` is the transaction's
//! real position in the *entire* block, votes included. Since
//! `catscope-geyser` filters out vote transactions before they ever reach
//! this guest (`if tx.is_vote { return Ok(None); }` in
//! `primitive/src/txproc.rs`), a gap in the sequence of indices this
//! guest sees is structurally indistinguishable between "a real
//! transaction got dropped" and "that position was just a vote,
//! correctly filtered" -- and since real Solana blocks are often 60-70%+
//! vote transactions, that index-gap approach was mostly measuring the
//! expected vote-filtering effect, not real data loss. That version's
//! "systemic ~60% missing" finding does not hold up; the method couldn't
//! have proven it either way.
//!
//! This version checks the real, decidable thing instead: it tracks every
//! distinct **signature** seen per slot (not index), so completeness can
//! be verified directly against a slot's real non-vote signature list
//! (pulled from `solana block <slot>`, filtering out `Vote111...`
//! instructions) -- a signature either was seen or it wasn't, no
//! index-based inference involved.
//!
//! Also sends a small number of real, self-signed native SOL transfers
//! (see `evaluate`) and tracks each one's real send->Transaction-lane
//! latency directly, independent of the passive slot-completeness check
//! above -- a real, live-user-caught follow-up question 2026-09-09: even
//! with confirmed partial coverage, is the Transaction lane's *indexing*
//! (correlating an incoming signature back to a specific pending send)
//! actually correct, and how fast is it when it does resolve? The
//! existing native-transfer latency tests can't answer this cleanly,
//! since their Account/LowLatency lane wins essentially every race
//! (live-verified all session), so their own Transaction-lane
//! confirmations almost never fire. This module watches *only* the
//! Transaction lane for its own sends -- no other lane competes for the
//! same signature at all.
use crate::{
    brain::midontxhealthv1::{
        message::{CustomMessageInbound, CustomMessageOutbound},
        Configuration,
    },
    catscope::witbot::{shooter::{Header, Tokenaccountv1}, transactionprocessor},
    event::SlotStatus,
    graph::{AccountId, CommitHook, Graph, LowLatencyAccountUpdate},
    log_warn,
    message::{InboundMesasgeHandler, MessageAction, MessageSend},
    txview::TransactionList,
    util::pubkey_from_account_id,
    wallet::Wallet,
};
use solana_sdk::{clock::Slot, pubkey::Pubkey, signature::Signature};
use solana_system_interface::instruction::transfer as system_transfer;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::{Duration, Instant},
};

#[derive(Debug, Default)]
pub(crate) struct State {
    /// Every distinct real signature this guest has observed for a given
    /// slot, across every non-vote transaction passing through
    /// `mid_on_tx` (votes never reach here at all -- filtered upstream in
    /// `catscope-geyser`, see this file's own top-level doc comment).
    /// Bounded to `StateHelper::WINDOW` slots (oldest evicted -- and
    /// logged -- first, a `VecDeque` for the same O(1) trim-oldest reason
    /// `slot_clock` uses one elsewhere in this codebase) so this can't
    /// grow unbounded over a long run.
    m_slot_signatures: VecDeque<(Slot, HashSet<Signature>)>,
    /// How many finalized slots have had their full signature set dumped
    /// to the log so far -- caps total log volume regardless of how long
    /// this bot runs (a busy mainnet slot can have hundreds of real
    /// non-vote transactions; dumping every one of them for every slot
    /// indefinitely would risk the same log-volume-driven stdio
    /// disconnects flagged elsewhere this session). Every slot still gets
    /// a real distinct-count logged either way -- only the expensive
    /// per-signature dump is capped.
    dumped_count: u32,
    /// This module's own signer, once the Go-side harness's `Wallet`
    /// message arrives (see `on_message`). `None` until then -- `evaluate`
    /// no-ops without it.
    o_owner: Option<AccountId>,
    /// How many of this module's own transfers have been sent so far --
    /// gates `evaluate` against `Self::NATIVE_TRANSFER_TARGET`.
    sent_count: u32,
    /// Wall-clock instant this module's most recent send actually went
    /// out -- gates `evaluate` from queuing the next send before enough
    /// real time has passed for the current one to plausibly resolve
    /// (real Solana slot cadence, not a guess -- see
    /// `StateHelper::SEND_COOLDOWN`). Deliberately *not* gated on waiting
    /// for the previous send's own confirmation, unlike every other
    /// native-transfer test this session -- the entire point here is that
    /// a real fraction of sends may never resolve via this lane at all
    /// (see this file's own top-level doc comment), so waiting on that
    /// would stall the whole test on the first one that doesn't.
    o_last_send: Option<Instant>,
    /// Signature -> real send `Instant`, for every transfer this module
    /// has sent whose Transaction-lane confirmation hasn't arrived (or
    /// timed out) yet. Removed once `mid_on_tx` sees the signature (a
    /// real hit, logged with the real latency) -- entries that are never
    /// removed this way are exactly the ones this test exists to count:
    /// a real transfer this guest's own Transaction lane never delivered.
    m_pending_tx: HashMap<Signature, Instant>,
}

/// `graph`/`wallet` are real (needed for the small number of real signed
/// transfers this module sends, see `evaluate`) -- `nonce`/`configuration`/
/// `q_msg`/`o_commit_slot` remain structurally required to match `mod.rs`'s
/// `helper()` construction (the same shape every other brain module uses)
/// but genuinely unused, since this module sends no outbound messages and
/// has no rooted-commit-specific behavior.
#[allow(dead_code)]
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
    /// How many recent slots' signature sets to keep open before
    /// finalizing (logging) and evicting the oldest -- generous relative
    /// to how far behind live the Transaction lane could plausibly still
    /// be delivering data for a given slot, while still bounding memory
    /// (a busy mainnet slot can have hundreds of real non-vote
    /// transactions).
    const WINDOW: usize = 64;

    /// How many finalized slots get their full signature set dumped to
    /// the log (see `State::dumped_count`'s doc comment for why this is
    /// capped) -- enough to spot-check a handful of real slots against
    /// `solana block <slot>` without risking real log-volume problems on
    /// a long run.
    const SIGNATURE_DUMP_LIMIT: u32 = 6;

    /// How many of this module's own real transfers to send -- small on
    /// purpose, this is a real-money mainnet test, not a full latency
    /// cycle: enough samples to see a real hit-rate and latency spread on
    /// the Transaction lane specifically, not a statistically airtight
    /// count.
    const NATIVE_TRANSFER_TARGET: u32 = 10;

    /// Lamports moved per send -- small (0.0002 SOL), sent straight back
    /// to the real parent/mothership wallet each time (see
    /// `crate::message::ENV_MOTHERSHIP_PUBKEY`) rather than round-
    /// tripping between two wallets like the full native-transfer tests
    /// do -- there's no reason to hold a second wallet open here, and
    /// sending straight back doubles as its own sweep, so there's nothing
    /// left stranded once the run ends.
    const TRANSFER_AMOUNT_LAMPORTS: u64 = 200_000;

    /// Minimum real wall-clock gap between sends -- roughly 2 real
    /// Solana slots. Deliberately *not* gated on the previous send's own
    /// confirmation (see `State::o_last_send`'s doc comment for why): a
    /// fixed cooldown still keeps sends from queuing faster than the
    /// chain could plausibly process them, without stalling the whole
    /// test on a send that never resolves via this lane.
    const SEND_COOLDOWN: Duration = Duration::from_millis(800);

    pub(crate) fn on_load(&mut self) {
        log_warn!(
            "midontxhealthv1: bot has been successfully uploaded to validator -- passive listen-only, checking mid_on_tx signature delivery completeness per slot"
        );
    }

    /// Not needed for this check (see module doc comment) -- kept only
    /// so `Event::SlotStatus` dispatch has somewhere to go.
    pub(crate) fn on_slot_status(&mut self, _slot: Slot, _status: SlotStatus) {}

    /// Not needed for this check -- dropped unread. This bot never
    /// touches account/token data.
    pub(crate) fn low_latency(&mut self, _llap: LowLatencyAccountUpdate) {}

    /// Records every non-vote transaction's real signature into its
    /// slot's distinct-signature set, then finalizes (logs and evicts)
    /// the oldest tracked slot once the window fills up. See
    /// `State::m_slot_signatures`'s doc comment for what this is
    /// actually checking, and `report_slot` for what gets logged.
    pub(crate) fn mid_on_tx(&mut self, mut transaction_list: TransactionList) {
        while let Some((tx, result)) = transaction_list.transaction() {
            // `Err` here just means this particular transaction failed
            // on-chain -- irrelevant to whether *we saw it at all*, which
            // is the only thing this check cares about. Skipped anyway
            // since a failed transaction carries no real landing slot to
            // key this by.
            let Ok(slot) = result else {
                continue;
            };
            let signature = Signature::from(*tx.signature);
            // Real send->Transaction-lane latency for this module's own
            // sends -- checked for *every* signature seen here, not just
            // ones we expect, since we have no other way to know in
            // advance which of our sends this particular batch might
            // resolve. See `State::m_pending_tx`'s doc comment.
            if let Some(sent_at) = self.state.m_pending_tx.remove(&signature) {
                log_warn!(
                    "midontxhealthv1: own transfer confirmed via Transaction lane -- signature={signature} slot={slot} latency={}µs",
                    sent_at.elapsed().as_micros(),
                );
            }
            if self
                .state
                .m_slot_signatures
                .back()
                .is_none_or(|&(s, _)| s < slot)
            {
                self.state
                    .m_slot_signatures
                    .push_back((slot, HashSet::default()));
                while Self::WINDOW < self.state.m_slot_signatures.len() {
                    if let Some((old_slot, set)) = self.state.m_slot_signatures.pop_front() {
                        self.report_slot(old_slot, &set);
                    }
                }
            }
            if let Some((_, set)) = self
                .state
                .m_slot_signatures
                .iter_mut()
                .rev()
                .find(|(s, _)| *s == slot)
            {
                set.insert(signature);
            }
        }
    }

    /// Sends this module's own small number of real, self-signed native
    /// transfers -- see this file's own top-level doc comment for why,
    /// and `State::o_owner`/`sent_count`/`o_last_send`/`m_pending_tx` for
    /// how progress and pending confirmations are tracked. No-ops until
    /// the Go-side harness's `Wallet` message arrives (`on_message`), and
    /// again once `NATIVE_TRANSFER_TARGET` sends have gone out.
    pub(crate) fn evaluate(&mut self) {
        let Some(owner) = self.state.o_owner else {
            return;
        };
        if Self::NATIVE_TRANSFER_TARGET <= self.state.sent_count {
            return;
        }
        if self
            .state
            .o_last_send
            .is_some_and(|t| t.elapsed() < Self::SEND_COOLDOWN)
        {
            return;
        }
        let Ok(mothership_str) = std::env::var(crate::message::ENV_MOTHERSHIP_PUBKEY) else {
            log_warn!(
                "midontxhealthv1: transfer send skipped -- {} env var not set",
                crate::message::ENV_MOTHERSHIP_PUBKEY,
            );
            return;
        };
        let Ok(mothership_pk) = Pubkey::try_from(mothership_str.as_str()) else {
            log_warn!(
                "midontxhealthv1: transfer send skipped -- couldn't parse {} ({mothership_str})",
                crate::message::ENV_MOTHERSHIP_PUBKEY,
            );
            return;
        };
        let Some(owner_pk) = pubkey_from_account_id(&owner) else {
            return;
        };
        let balance = self.wallet.balance_sol(&owner).unwrap_or(0);
        // A real, honest reserve for this send's own fee -- skip
        // (not panic) if the wallet's run dry rather than send a
        // transaction that can't possibly land.
        if balance < Self::TRANSFER_AMOUNT_LAMPORTS + 10_000 {
            log_warn!(
                "midontxhealthv1: transfer send skipped -- balance {balance} lamports below {} needed",
                Self::TRANSFER_AMOUNT_LAMPORTS + 10_000,
            );
            return;
        }
        self.wallet.require_signer(owner);
        self.wallet.append_ix(
            system_transfer(&owner_pk, &mothership_pk, Self::TRANSFER_AMOUNT_LAMPORTS),
            5_000,
        );
        let Some((signature, tx_bytes)) = self.wallet.assemble() else {
            return;
        };
        // Owned copy -- `assemble`'s `&[u8]` borrows `self.wallet`, and
        // `transactionprocessor::send`'s real signature wants an owned
        // buffer anyway (same pattern every other real send in this
        // codebase uses).
        let tx_bytes = tx_bytes.to_vec();
        match transactionprocessor::send(signature.as_array(), &tx_bytes) {
            Ok(_) => {
                log_warn!("midontxhealthv1: sent transfer {signature}");
                self.state.m_pending_tx.insert(signature, Instant::now());
                self.state.sent_count += 1;
                self.state.o_last_send = Some(Instant::now());
            }
            Err(e) => {
                log_warn!("midontxhealthv1: failed to send transfer: {e:?}");
            }
        }
    }

    /// Logs one slot's real distinct-signature count once it's finalized
    /// (aged out of `State::m_slot_signatures`'s tracking window) -- every
    /// slot gets this. For the first `SIGNATURE_DUMP_LIMIT` finalized
    /// slots, also dumps every individual signature seen, one per line,
    /// so a real slot can be spot-checked directly against
    /// `solana block <slot>`'s own real non-vote signature list --
    /// a signature either was seen or it wasn't, no index-based
    /// inference involved (see this file's own top-level doc comment for
    /// why that matters).
    fn report_slot(&mut self, slot: Slot, set: &HashSet<Signature>) {
        let count = set.len();
        log_warn!(
            "midontxhealthv1: slot {slot} -- {count} distinct non-vote signature(s) seen"
        );
        if self.state.dumped_count < Self::SIGNATURE_DUMP_LIMIT {
            self.state.dumped_count += 1;
            for sig in set {
                log_warn!("midontxhealthv1: slot {slot} saw signature {sig}");
            }
            log_warn!(
                "midontxhealthv1: slot {slot} -- end of signature dump ({count} total, dump {}/{})",
                self.state.dumped_count,
                Self::SIGNATURE_DUMP_LIMIT,
            );
        }
    }
}

impl<'a> InboundMesasgeHandler<Configuration, CustomMessageInbound, CustomMessageOutbound>
    for StateHelper<'a>
{
    /// Only the real `Wallet` key is handled -- everything else this
    /// module has no use for (`Ping`/`AdjustConfiguration`/`Shutdown`,
    /// and the `Blank` fallback `message`'s deserializer produces for
    /// anything unrecognized) is discarded.
    fn on_message(&mut self, action: MessageAction<Configuration, CustomMessageInbound>) {
        if let MessageAction::Custom(CustomMessageInbound::Wallet(rc_keypair)) = action {
            match self.wallet.append_key(rc_keypair, self.graph) {
                Ok(owner) => {
                    self.wallet.set_payer(owner);
                    self.state.o_owner = Some(owner);
                    log_warn!("midontxhealthv1: got wallet, owner={owner}");
                }
                Err(e) => log_warn!("midontxhealthv1: failed to register wallet key: {e:?}"),
            }
        }
    }

    fn message_send(&mut self, _message: MessageSend<CustomMessageOutbound>) {}
}

impl<'a> CommitHook for StateHelper<'a> {
    fn start(&mut self, _slot: Slot) {}
    fn on_account(&mut self, _header: &Header, _body: &[u8]) {}
    fn on_token(&mut self, _token_account: &Tokenaccountv1) {}
    fn finish(&mut self) {}
}
