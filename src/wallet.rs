use crate::{
    atl_config,
    bundler_message::BundlerTipUpdate,
    catscope::witbot::{
        shooter::{Header, Tokenaccountv1},
        transactionprocessor,
    },
    err::CatscopeGuestError,
    graph::{AccountId, Graph, Lamports, Subscription, SubscriptionQueue, SubscriptionRequest},
    log_error, log_warn,
    token::TokenDatabase,
    tx::ComputeUnit,
    util::{account_id_from_pubkey, pubkey_from_account_id, rc_unlock},
};
use bincode;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_nonce::{state::State as NonceAccountState, versions::Versions as NonceVersions};
use solana_sdk::{
    hash::Hash,
    message::{v0, Instruction, VersionedMessage},
    pubkey::Pubkey,
    rent::Rent,
    signature::{Keypair, Signature},
    signer::Signer,
    transaction::{Transaction, VersionedTransaction},
};
use solana_sdk_ids::system_program::ID as SystemProgramID;
use spl_associated_token_account::{
    get_associated_token_address, instruction::create_associated_token_account_idempotent,
};
use std::{
    cell::UnsafeCell,
    collections::{HashMap, HashSet, VecDeque},
    hash::BuildHasherDefault,
    rc::Rc,
};
use twox_hash::XxHash64;

#[derive(Clone, Copy, Default)]
pub enum PriorityLevel {
    #[default]
    None,
    Medium,
    High,
    /// Raw micro-lamports per compute unit.
    Custom(u64),
}

impl From<PriorityLevel> for u64 {
    fn from(level: PriorityLevel) -> u64 {
        match level {
            PriorityLevel::None => 0,
            PriorityLevel::Medium => 10_000,
            PriorityLevel::High => 100_000,
            PriorityLevel::Custom(v) => v,
        }
    }
}

/// SPL Token-2022 program -- not a dependency of this crate (only classic
/// SPL Token is used anywhere else in it), hardcoded the same way other
/// well-known program IDs are elsewhere in this codebase (e.g.
/// `dex::solend::SOLEND_PROGRAM_ID`). Only referenced by
/// [`ALT_EXCLUDED_PROGRAMS`].
const TOKEN_2022_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

/// Well-known, always-resident program IDs excluded from
/// [`Wallet::tally_account_usage`] (and therefore never reported via
/// `MessageSend::CommonAddressUpdate`) -- they're only 32 bytes each and
/// already present in virtually every transaction regardless of which
/// protocol it's for, so a scarce lookup-table slot is better spent on
/// accounts that vary per instruction. Mirrors the exclusion set
/// `optimizer`'s now-removed RPC-based ranking used to apply on the Go
/// side (`cmd/alt.go`'s former `fetchRankedAccounts`).
const ALT_EXCLUDED_PROGRAMS: [Pubkey; 6] = [
    SystemProgramID,
    spl_token::ID,
    TOKEN_2022_PROGRAM_ID,
    spl_associated_token_account::ID,
    solana_sdk_ids::compute_budget::ID,
    solana_sdk_ids::address_lookup_table::ID,
];

/// Deterministic seed for this wallet's own durable-nonce account (see
/// [`Wallet::send_bundler_pair`]) -- well under Solana's 32-byte
/// `create_with_seed` limit. Mirrors `dex::solend::OBLIGATION_SEED`'s
/// exact role, just for a System-Program-owned account instead of a
/// foreign program's.
const BUNDLER_NONCE_SEED: &str = "bundler-nonce";

pub struct Wallet {
    m_key: HashMap<AccountId, SignerStatus>,
    /// Each queued instruction paired with its own compute-unit cost and a
    /// `group_start` marker -- needed (not just a running total) so
    /// `assemble()` can split the queue across multiple transactions and
    /// still set an accurate `set_compute_unit_limit` on each one
    /// individually. `group_start=false` forbids `assemble()` from ever
    /// placing a transaction boundary immediately before that
    /// instruction -- see [`Self::begin_atomic_group`].
    q_ix: VecDeque<(Instruction, ComputeUnit, bool)>,
    /// Running total across the *entire* queue -- kept for [`Self::cu()`],
    /// which callers (e.g. `arbv1`'s scratch-wallet profit estimate) use to
    /// reason about a whole planned instruction set before anything is
    /// ever assembled/split.
    compute: ComputeUnit,
    token: TokenDatabase,
    sys_id: AccountId,
    tx_data: Box<[u8; 4 * 1024]>,
    payer: Option<AccountId>,
    m_cache_pubkey: HashMap<AccountId, Pubkey>,
    hs_required: HashSet<AccountId>,
    /// ALT account ID → ordered account list (index in the ALT = position in Vec)
    l_alt: Vec<AddressLookupTable>,
    priority_level: PriorityLevel,
    /// Mirrors whether `priority_level` is currently `High` -- set
    /// alongside it by [`Wallet::set_priority_fee`], never independently
    /// (2026-08-28, explicit: "If the priority is set to high in
    /// Wallet::set_priority_fee(), then set the landing priority to
    /// High"). [`Wallet::drain_and_send`] checks this to decide whether
    /// to try [`Wallet::send_bundler_pair`] automatically before falling
    /// back to its ordinary path.
    desired_landing_high: bool,
    /// Subscriptions for this wallet's own SPL token accounts (USDC/wSOL/
    /// any other tracked mint's ATA) -- kept alive here so dropping them
    /// doesn't cancel the subscription. Unlike `m_key`'s per-signer
    /// `Subscription` (correlated 1:1 with a specific signer account),
    /// these don't need per-mint bookkeeping: `TokenDatabase::on_token`
    /// matches incoming updates by the account data's own `(owner,
    /// mint)` fields, not by remembering which subscription produced
    /// which account -- a flat `Vec` is enough. Real, live-observed
    /// motivation: before this existed, nothing in this codebase ever
    /// subscribed to a wallet's own ATAs at all (`derive_ata`/
    /// `append_create_ata` only compute the address / build an
    /// instruction) -- `append_key`'s depth:1 subscription on the
    /// wallet's root account was deliberately narrowed
    /// (`filter_weight: 0`) away from pulling in "obligations, farm
    /// state, etc.", which also excluded the wallet's own token
    /// accounts. Confirmed live: a real swap succeeded on-chain
    /// (verified via `solana confirm -v`, 22+ USDC landed in the
    /// wallet's real ATA) while `TokenDatabase`/`current_usdc_value`
    /// stayed at 0 the entire time, since no `on_token`/low-latency
    /// update for that ATA was ever received.
    l_ata_sub: Vec<Subscription>,
    /// Cumulative-since-process-start count of how often each non-signer
    /// account has appeared across every instruction ever assembled into
    /// a transaction by this wallet -- tallied in [`Self::assemble`] (the
    /// same account-meta scan that already looks for ALT matches).
    /// Signer accounts are excluded: never ALT-eligible. Feeds
    /// [`Self::top_account_usage`], the only reader, which brain modules
    /// periodically report via `MessageSend::CommonAddressUpdate` so
    /// `optimizer alt` can rank real usage without an RPC history scan.
    m_account_usage: HashMap<AccountId, (Pubkey, u32), BuildHasherDefault<XxHash64>>,
    /// Live bundler tip state (address list + up/down + tip-size
    /// distribution), keyed by bundler code (see `bundler_config::BUNDLER`'s
    /// doc comment for the u8 convention: 0=default/none, 1=astralane,
    /// 2=jito, matching `optimizer/bundler.BundlerCode`). Populated by
    /// [`Self::apply_bundler_tip_update`], fed by the optimizer's periodic
    /// `bundler.Tip()`/`Distribution()` poll over the shared
    /// `MessageAction` wire path
    /// (`bundler_message::COMMON_KEY_FLAG_BUNDLER_TIP_UPDATE`). Read by
    /// [`Self::select_tip_account`]/[`Self::tip_lamports`]/
    /// [`Self::append_bundler_tip`] when a caller wants to tip a bundler
    /// for the current transaction.
    m_bundler_tip: HashMap<u8, BundlerTipState, BuildHasherDefault<XxHash64>>,
    /// Every tip `AccountId` ever seen across every
    /// [`Self::apply_bundler_tip_update`] call -- lets that method skip
    /// re-subscribing an address it already subscribed on an earlier
    /// update (the tip address list is close to static; only the
    /// distribution changes on most updates, which arrive far more often
    /// than the address list does).
    hs_known_tip_accounts: HashSet<AccountId>,
    /// Keeps every tip-account `Subscription`
    /// [`Self::apply_bundler_tip_update`] creates alive (a `Subscription`
    /// cancels on drop) -- same "just don't let it drop, no per-account
    /// correlation needed" role [`Self::l_ata_sub`] already plays for
    /// this wallet's own ATAs.
    l_tip_sub: Vec<Subscription>,
    /// This wallet's own durable-nonce account (see
    /// [`Self::send_bundler_pair`]) and its last-known readiness -- `None`
    /// until [`Self::nonce_subscribe_request`] has been called once, at
    /// wallet-key-set time. Updated by [`Self::on_account`] whenever a
    /// real account update for it arrives.
    o_nonce: Option<NonceInfo>,
    /// Keeps the durable-nonce account's `Subscription` alive -- same
    /// "just don't let it drop" role [`Self::l_ata_sub`] plays.
    o_nonce_sub: Option<Subscription>,
    /// `true` while an atomic group is open (see [`Self::begin_atomic_group`]).
    in_atomic_group: bool,
    /// `true` once the first instruction of the currently-open atomic
    /// group has been appended -- see [`Self::append_ix`]'s use of this
    /// to decide each instruction's `group_start` marker.
    atomic_group_started: bool,
}

struct SignerStatus {
    key: Rc<UnsafeCell<Keypair>>,
    header: Header,
    sub: Subscription,
}

/// One bundler's live tip state -- see [`Wallet::m_bundler_tip`].
#[derive(Default)]
struct BundlerTipState {
    up: bool,
    tip_account_ids: Vec<AccountId>,
    /// 25th/50th/75th/95th/99th percentile lamports required to land in
    /// a block, same order as `optimizer/bundler.Bundler::Distribution`'s
    /// `[5]graph.Lamports`.
    distribution: [u64; 5],
}

/// This wallet's durable-nonce account and its last-known readiness --
/// see [`Wallet::o_nonce`].
struct NonceInfo {
    account_id: AccountId,
    state: NonceReadiness,
}

/// Live readiness of [`NonceInfo`]'s account. See
/// [`Wallet::ensure_bundler_nonce_created`]/[`Wallet::send_bundler_pair`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NonceReadiness {
    /// Subscribed, but no real account read has come back yet. Treated
    /// as distinct from `Uninitialized` deliberately:
    /// `ensure_bundler_nonce_created` only acts on a *confirmed*
    /// `Uninitialized` read, not this state, so a fresh subscription
    /// can't race a duplicate create attempt against an account that
    /// actually already exists but whose first update just hasn't
    /// arrived yet.
    Unknown,
    /// A real read confirmed the account doesn't exist yet, or exists
    /// but isn't `Initialized`.
    Uninitialized,
    /// `ensure_bundler_nonce_created` has already queued the real
    /// create-nonce transaction for this account and is waiting for
    /// confirmation -- self-latching so repeated calls (this is now
    /// called unconditionally every `evaluate()` tick by every brain
    /// module, not just a manual trigger) don't requeue a duplicate
    /// create transaction every tick until the first one lands. Never
    /// produced by `decode_nonce_readiness` (real on-chain bytes only
    /// ever decode to `Uninitialized`/`Ready`) -- purely a client-side
    /// "don't ask again this session" marker. If the queued create
    /// somehow never lands, this wallet's nonce just never becomes
    /// `Ready` and Astralane landing (`send_bundler_pair`) keeps
    /// declining -- no retry, same best-effort philosophy as everything
    /// else in the bundler-tip system.
    CreationQueued,
    /// Ready to use -- carries the durable nonce's current value, used
    /// directly as a nonce transaction's `recent_blockhash`.
    Ready(Hash),
}

impl Default for Wallet {
    fn default() -> Self {
        Self::new()
    }
}
#[derive(Debug, Clone)]
pub struct AddressLookupTable {
    key: AccountId,
    key_pubkey: Pubkey,
    m_account: HashMap<AccountId, u8, BuildHasherDefault<XxHash64>>,
    l_pubkey: Vec<Pubkey>,
}
impl TryFrom<&[u8]> for AddressLookupTable {
    type Error = CatscopeGuestError;

    fn try_from(data: &[u8]) -> Result<Self, Self::Error> {
        let pubkey_len = std::mem::size_of::<Pubkey>();
        if data.is_empty() {
            return Err(CatscopeGuestError::InsufficientBuffer);
        }
        if !data.len().is_multiple_of(pubkey_len) {
            return Err(CatscopeGuestError::InsufficientBuffer);
        }
        let n = data.len() / pubkey_len;
        if n < 2 || u8::MAX as usize <= n {
            return Err(CatscopeGuestError::InsufficientBuffer);
        }
        let mut i = 0;
        let mut subbuf = &data[i..(i + pubkey_len)];
        i += pubkey_len;
        let key_pubkey = Pubkey::new_from_array(subbuf.try_into().unwrap());
        let key = account_id_from_pubkey(&key_pubkey);
        let mut m_account = HashMap::with_capacity_and_hasher(n - 1, BuildHasherDefault::default());
        let mut l_pubkey = Vec::with_capacity(n - 1);
        let mut k: u8 = 0;
        while i < data.len() {
            subbuf = &data[i..(i + pubkey_len)];
            i += pubkey_len;
            let pubkey = Pubkey::new_from_array(subbuf.try_into().unwrap());
            m_account.insert(account_id_from_pubkey(&pubkey), k);
            l_pubkey.push(pubkey);
            k += 1;
        }
        Ok(Self {
            key,
            key_pubkey,
            m_account,
            l_pubkey,
        })
    }
}

impl AddressLookupTable {
    pub fn key(&self) -> &AccountId {
        &self.key
    }
    pub fn map(&self) -> &HashMap<AccountId, u8, BuildHasherDefault<XxHash64>> {
        &self.m_account
    }
    pub fn load_default() -> Result<Vec<Self>, CatscopeGuestError> {
        if atl_config::ADDRESS_LOOKUP_TABLES.is_empty() {
            return Ok(vec![]);
        }
        let mut l_alt = Vec::with_capacity(atl_config::ADDRESS_LOOKUP_TABLES.len());
        for (key_pk, l_pk) in atl_config::ADDRESS_LOOKUP_TABLES {
            let key_pubkey = Pubkey::new_from_array(*key_pk);
            let key = account_id_from_pubkey(&key_pubkey);
            let n = l_pk.len();
            if n == 0 {
                return Err(CatscopeGuestError::InsufficientBuffer);
            }
            let mut m_account = HashMap::with_capacity_and_hasher(n, BuildHasherDefault::default());
            let mut l_pubkey = Vec::with_capacity(n);
            let mut k: u8 = 0;
            for pk in l_pk.iter() {
                let pubkey = Pubkey::new_from_array(*pk);
                let account_id = account_id_from_pubkey(&pubkey);
                m_account.insert(account_id, k);
                l_pubkey.push(pubkey);
                k += 1;
            }
            l_alt.push(Self {
                key,
                key_pubkey,
                m_account,
                l_pubkey,
            });
        }
        Ok(l_alt)
    }
}

impl Wallet {
    pub fn new() -> Self {
        let l_alt = AddressLookupTable::load_default().expect("address lookup table");
        Self {
            m_cache_pubkey: HashMap::default(),
            payer: None,
            token: TokenDatabase::default(),
            tx_data: Box::new([0u8; 4 * 1024]),
            m_key: HashMap::default(),
            q_ix: VecDeque::default(),
            compute: 0,
            sys_id: account_id_from_pubkey(&SystemProgramID),
            hs_required: HashSet::new(),
            l_alt,
            priority_level: PriorityLevel::default(),
            desired_landing_high: false,
            l_ata_sub: Vec::new(),
            m_account_usage: HashMap::default(),
            m_bundler_tip: HashMap::default(),
            hs_known_tip_accounts: HashSet::new(),
            l_tip_sub: Vec::new(),
            o_nonce: None,
            o_nonce_sub: None,
            in_atomic_group: false,
            atomic_group_started: false,
        }
    }

    /// Opens an atomic group: every `append_ix` call (directly, or
    /// indirectly through helpers like `append_create_ata` or a
    /// DEX-specific hop-execution function) from now until the matching
    /// [`Self::end_atomic_group`] gets glued to the instruction queued
    /// immediately before it -- `assemble()` will never place a
    /// transaction boundary in the middle of the group, only immediately
    /// before its first instruction or after its last one. Use around a
    /// set of instructions that only make sense together (e.g. an ATA's
    /// `CreateIdempotent` and the swap that reads it) -- real,
    /// live-confirmed motivation: without this, `assemble()`'s
    /// byte-size-only splitter could (and did) cut a swap into a
    /// different transaction than its own destination-ATA-creation
    /// instruction, and the two transactions have no ordering/
    /// confirmation guarantee between them, so the swap sometimes landed
    /// first and reverted with `AccountNotInitialized` (Anchor error
    /// 3012 / `0xbc4`) -- observed on a real mainnet Raydium CLMM swap
    /// inside a 4-hop `execute_spot_leg` route, where `Wallet::assemble`
    /// split hop N's swap away from hop (N+1)'s `CreateIdempotent` calls.
    /// Nesting is not supported -- callers must not call this again
    /// before the matching `end_atomic_group`.
    pub fn begin_atomic_group(&mut self) {
        debug_assert!(!self.in_atomic_group, "atomic groups do not nest");
        self.in_atomic_group = true;
        self.atomic_group_started = false;
    }

    /// Closes the atomic group opened by [`Self::begin_atomic_group`].
    pub fn end_atomic_group(&mut self) {
        self.in_atomic_group = false;
        self.atomic_group_started = false;
    }

    /// Set the priority fee for the next assembled transaction. Call
    /// this at the start of each evaluate cycle.
    ///
    /// Also sets/clears [`Self::desired_landing_high`] to match whether
    /// `level` is `High` (2026-08-28, explicit: "If the priority is set
    /// to high in Wallet::set_priority_fee(), then set the landing
    /// priority to High") -- [`Self::drain_and_send`] uses that to decide
    /// whether to try real Astralane landing ([`Self::send_bundler_pair`])
    /// automatically for whatever gets queued next, rather than requiring
    /// a separate opt-in call. Setting priority back down to `Medium`/
    /// `None`/`Custom` (every brain module's `evaluate()` already does
    /// this once per tick before anything urgent overrides it) clears the
    /// flag the same way, so it never lingers past the tick that actually
    /// wanted it.
    pub fn set_priority_fee(&mut self, level: PriorityLevel) {
        self.desired_landing_high = matches!(level, PriorityLevel::High);
        self.priority_level = level;
    }

    /// Set priority fee from a raw micro-lamports-per-CU value.
    pub fn set_priority_fee_micro_lamports(&mut self, micro_lamports_per_cu: u64) {
        self.priority_level = PriorityLevel::Custom(micro_lamports_per_cu);
    }

    /// The real, exact priority-fee lamports a transaction using `compute`
    /// compute units pays at the *currently set* priority level -- the
    /// same math `build_ix_list`/`build_bundler_variant` apply via
    /// `ComputeBudgetInstruction::set_compute_unit_price`, factored out
    /// here for callers that need to predict a transaction's exact real
    /// cost up front rather than estimate it. `div_ceil` because Solana's
    /// own fee calculation rounds the microlamports-per-CU product *up*
    /// to the next lamport, not down.
    ///
    /// Real, live-verified 2026-09-03: `testperplatencyv1`'s balance-
    /// arrival check used to only budget for the flat per-signature base
    /// fee, missing this entirely -- at `PriorityLevel::Medium` (10,000
    /// microlamports/CU) over a 5,000 CU transfer, that's exactly 50
    /// lamports quietly uncounted, which made the fee-payer's own
    /// balance permanently fall 50 lamports short of the threshold the
    /// old code expected, so an owner-recipient transfer's Account/Commit
    /// lanes could never win the confirmation race -- only the fee-
    /// independent Transaction (signature) lane ever could.
    pub fn priority_fee_lamports(&self, compute: ComputeUnit) -> u64 {
        let micro_lamports_per_cu = u64::from(self.priority_level);
        (u64::from(compute) * micro_lamports_per_cu).div_ceil(1_000_000)
    }

    pub fn require_signer(&mut self, account_id: AccountId) {
        self.hs_required.insert(account_id);
    }

    pub fn set_payer(&mut self, payer: AccountId) {
        self.payer = Some(payer);
    }

    /// Add a private key to the wallet.
    /// This also includes doing a graph subscription to get Lamport updates.
    pub fn append_key(
        &mut self,
        keypair: Rc<UnsafeCell<Keypair>>,
        g1: &mut Graph,
    ) -> Result<AccountId, CatscopeGuestError> {
        let key = rc_unlock(&keypair);
        let pubkey = key.pubkey();
        let signer = account_id_from_pubkey(&pubkey);
        // Root account only -- this wallet's own SOL balance (`on_account`,
        // matched against `self.m_key`). Token account balances do NOT come
        // through this subscription at all; they're a separate pipeline
        // (`on_token`, fed by `Tokenaccountv1` messages via `low_latency`).
        // `depth:
        // an "owner ->" edge to (obligations, farm state, etc. -- whatever
        // real positions this wallet happens to hold across every protocol
        // it's ever touched), which this subscription never needed and
        // made it slow/unbounded for a wallet with real history instead of
        // the single, cheap root-account lookup it should be.
        assert_ne!(self.sys_id, signer);
        assert_ne!(0, signer);
        let sub = g1.subscribe(crate::graph::SubscriptionRequest {
            root: signer,
            filter_weight: 0,
            depth: 1,
        })?;

        self.m_key.insert(
            signer,
            SignerStatus {
                key: keypair,
                header: Header {
                    slot: 0,
                    version: 0,
                    lamports: 0,
                    accountid: 0,
                    owner: 0,
                    datasize: 0,
                },
                sub,
            },
        );
        Ok(signer)
    }

    pub fn has_key(&self, l_account_id: &[AccountId]) -> Option<&Header> {
        for account_id in l_account_id {
            if let Some(ss) = self.m_key.get(account_id) {
                return Some(&ss.header);
            }
        }
        None
    }

    pub fn payer_pubkey(&self) -> Option<Pubkey> {
        let payer_id = self.payer?;
        let ss = self.m_key.get(&payer_id)?;
        let keypair = rc_unlock(&ss.key);
        Some(keypair.pubkey())
    }

    /// Update the signer system account status (includes SOL balance).
    pub fn on_token(&mut self, a: &Tokenaccountv1, is_final: bool) -> bool {
        let x = self.m_key.contains_key(&a.owner);
        if x {
            self.token.on_token(a, is_final);
        }
        x
    }

    pub fn token(&self) -> &TokenDatabase {
        &self.token
    }
    pub fn token_mut(&mut self) -> &mut TokenDatabase {
        &mut self.token
    }

    /// Update the signer system account status (SOL balance) and, if
    /// `header` is this wallet's own durable-nonce account (see
    /// [`Self::o_nonce`]), its decoded readiness. `body` is only used for
    /// the nonce-account case -- the signer-balance case only ever needed
    /// the header.
    pub fn on_account(&mut self, header: &Header, body: &[u8]) -> bool {
        if header.owner != self.sys_id {
            return false;
        }
        let mut handled = false;
        if let Some(status) = self.m_key.get_mut(&header.accountid) {
            log_warn!(
                "on_account - wallet - 1 - pubkey {}; slot {}; lamports {}",
                header.accountid,
                header.slot,
                header.lamports
            );
            status.header = *header;
            handled = true;
        }
        if let Some(info) = self.o_nonce.as_mut() {
            if info.account_id == header.accountid {
                let decoded = decode_nonce_readiness(body);
                // Don't let a stale pre-creation read undo
                // `ensure_bundler_nonce_created`'s `CreationQueued` latch
                // (real risk: subscriptions can deliver many updates per
                // second, and the real create transaction can easily
                // take longer than that to confirm -- an update that
                // still reads `Uninitialized` in that window would
                // otherwise reset the latch and let the next tick queue
                // a duplicate create). Only this specific regression is
                // suppressed; `CreationQueued -> Ready` (the real
                // confirmation) and every other transition still apply.
                if !(info.state == NonceReadiness::CreationQueued
                    && decoded == NonceReadiness::Uninitialized)
                {
                    info.state = decoded;
                }
                handled = true;
            }
        }
        handled
    }

    pub fn balance_sol(&self, signer_account_id: &AccountId) -> Option<Lamports> {
        let ss = self.m_key.get(signer_account_id)?;
        Some(ss.header.lamports)
    }
    fn pubkey_from_account_id(&mut self, account_id: &AccountId) -> Option<Pubkey> {
        if let Some(pubkey) = self.m_cache_pubkey.get(account_id) {
            Some(*pubkey)
        } else {
            let pubkey = pubkey_from_account_id(account_id)?;
            self.m_cache_pubkey.insert(*account_id, pubkey);
            Some(pubkey)
        }
    }
    /// Derive the ATA address for `owner`+`mint` -- pure computation, does
    /// NOT mutate `self` or queue any instruction (unlike
    /// `append_create_ata` below), so it's safe to call against a scratch
    /// `Wallet` purely for planning purposes (see
    /// `brain::arbv1::state::StateHelper::build_execution_plan`) without
    /// any risk of polluting a real instruction queue. Doesn't assume the
    /// ATA needs creating -- just derives the address. Calls the
    /// `pubkey_from_account_id` WIT host import, same as everything else
    /// in this file -- not callable from a native `cargo test` binary.
    pub fn derive_ata(&self, owner: AccountId, mint: AccountId) -> Option<AccountId> {
        let owner_pubkey = pubkey_from_account_id(&owner)?;
        let mint_pubkey = pubkey_from_account_id(&mint)?;
        let ata_address = get_associated_token_address(&owner_pubkey, &mint_pubkey);
        Some(account_id_from_pubkey(&ata_address))
    }

    /// Derives (not subscribes -- no host call besides `derive_ata`'s
    /// own address resolution, both cache-backed via
    /// `util::PubkeyAccountIdCache`) the `SubscriptionRequest` for
    /// `owner`'s ATA of `mint`. Pair with [`Self::keep_ata_subscriptions`]
    /// once the request has actually been sent (typically batched
    /// together with other subscribe requests in the same call, same
    /// shape as every other `authority_subscribe_requests`/`apply_*` split
    /// in this codebase) -- see [`Self::l_ata_sub`]'s doc comment for why
    /// this is the only way a wallet's own SPL token balance (USDC, wSOL,
    /// any tracked mint) is actually tracked at all.
    pub fn ata_subscribe_request(
        &self,
        owner: AccountId,
        mint: AccountId,
    ) -> Option<SubscriptionRequest> {
        let ata = self.derive_ata(owner, mint)?;
        Some(SubscriptionRequest {
            root: ata,
            filter_weight: 0,
            depth: 1,
        })
    }

    /// Keeps `subs` alive so their subscriptions stay active -- see
    /// [`Self::l_ata_sub`]'s own doc comment for why no per-mint
    /// correlation is needed here, unlike `m_key`'s per-signer
    /// `Subscription`.
    pub fn keep_ata_subscriptions(&mut self, subs: Vec<Subscription>) {
        self.l_ata_sub.extend(subs);
    }

    /// The deterministic, `create_account_with_seed`-derived address of
    /// `owner`'s durable-nonce account (see [`Self::send_bundler_pair`])
    /// -- mirrors `dex::solend::obligation_address`'s exact pattern, just
    /// seeded off the wallet's own pubkey with the System Program as
    /// owner instead of a foreign program, so no separate keypair ever
    /// needs to be generated or persisted, and the address is
    /// deterministic across bot restarts.
    fn nonce_address(owner: &Pubkey) -> Pubkey {
        Pubkey::create_with_seed(owner, BUNDLER_NONCE_SEED, &SystemProgramID)
            .expect("BUNDLER_NONCE_SEED is a valid create-with-seed seed")
    }

    /// Derives (via [`Self::nonce_address`]) and records this wallet's
    /// durable-nonce account for tracking, returning the
    /// `SubscriptionRequest` needed to actually receive its live state --
    /// pair with [`SubscriptionQueue::subscribe_now`]/
    /// [`Self::keep_nonce_subscription`], same shape as
    /// [`Self::ata_subscribe_request`]/[`Self::keep_ata_subscriptions`].
    /// Call once, at wallet-key-set time.
    pub fn nonce_subscribe_request(&mut self, owner: AccountId) -> Option<SubscriptionRequest> {
        let owner_pubkey = pubkey_from_account_id(&owner)?;
        let nonce_pubkey = Self::nonce_address(&owner_pubkey);
        let account_id = account_id_from_pubkey(&nonce_pubkey);
        self.o_nonce = Some(NonceInfo {
            account_id,
            state: NonceReadiness::Unknown,
        });
        Some(SubscriptionRequest {
            root: account_id,
            filter_weight: 0,
            depth: 1,
        })
    }

    /// Keeps the durable-nonce account's `Subscription` alive -- pair
    /// with [`Self::nonce_subscribe_request`].
    pub fn keep_nonce_subscription(&mut self, sub: Subscription) {
        self.o_nonce_sub = Some(sub);
    }

    /// `Some(blockhash)` iff this wallet's durable-nonce account is
    /// confirmed `Initialized` on-chain -- the value to use as a nonce
    /// transaction's `recent_blockhash` field (see
    /// [`Self::send_bundler_pair`]).
    pub fn nonce_ready(&self) -> Option<Hash> {
        match self.o_nonce.as_ref()?.state {
            NonceReadiness::Ready(blockhash) => Some(blockhash),
            NonceReadiness::Unknown
            | NonceReadiness::Uninitialized
            | NonceReadiness::CreationQueued => None,
        }
    }

    /// If this wallet's durable-nonce account is confirmed missing/
    /// uninitialized (a real account read already came back that way --
    /// see [`NonceReadiness::Unknown`]'s doc comment for why an
    /// unconfirmed state doesn't qualify), appends the real
    /// `create_account_with_seed` + `InitializeNonceAccount` instructions
    /// (via the normal `assemble()`/`drain_and_send()` path -- a
    /// one-time, ordinary transaction, not part of any bundler pair),
    /// latches the state to [`NonceReadiness::CreationQueued`] so it
    /// won't do this again until either a real confirmation flips it to
    /// `Ready` or the process restarts, and returns `true` (caller should
    /// wait for confirmation, not attempt [`Self::send_bundler_pair`]
    /// this tick). `false` if there's nothing to do: already `Ready` or
    /// `CreationQueued`, or still `Unknown`/never subscribed. Self-
    /// latching makes this safe to call unconditionally every
    /// `evaluate()` tick (2026-08-28, explicit: every brain module now
    /// does exactly that, to bootstrap the nonce automatically at wallet
    /// load time instead of requiring a manual trigger) -- without it,
    /// every tick between "first confirmed Uninitialized read" and "the
    /// create transaction's own confirmation lands" would queue another
    /// duplicate create transaction.
    pub fn ensure_bundler_nonce_created(&mut self, owner: AccountId) -> bool {
        let Some(info) = self.o_nonce.as_ref() else {
            return false;
        };
        if info.state != NonceReadiness::Uninitialized {
            return false;
        }
        // Resolve the owner's pubkey *before* latching -- a failure here
        // means nothing gets queued, so the latch must not fire either,
        // or a real Uninitialized nonce would get stuck permanently
        // unactionable.
        let Some(owner_pubkey) = pubkey_from_account_id(&owner) else {
            return false;
        };
        self.o_nonce.as_mut().unwrap().state = NonceReadiness::CreationQueued;
        let nonce_pubkey = Self::nonce_address(&owner_pubkey);
        let lamports = Rent::default().minimum_balance(NonceAccountState::size());
        self.require_signer(owner);
        for ix in solana_system_interface::instruction::create_nonce_account_with_seed(
            &owner_pubkey,
            &nonce_pubkey,
            &owner_pubkey,
            BUNDLER_NONCE_SEED,
            &owner_pubkey,
            lamports,
        ) {
            self.append_ix(ix, Self::NONCE_CREATE_CU);
        }
        true
    }

    /// Derive the ATA for `owner`+`mint`, append a `CreateIdempotent` instruction,
    /// and return the ATA pubkey. Returns `None` if no payer is set.
    pub fn append_create_ata(&mut self, owner: AccountId, mint: AccountId) -> Option<AccountId> {
        let owner_pubkey = self.pubkey_from_account_id(&owner)?;
        let mint_pubkey = self.pubkey_from_account_id(&mint)?;
        let ix = make_ata_instruction(&owner_pubkey, &owner_pubkey, &mint_pubkey);
        let ata_address: Pubkey = get_associated_token_address(&owner_pubkey, &mint_pubkey);
        self.require_signer(owner);
        // Was 5_000 -- real, live-confirmed `ComputationalBudgetExceeded`
        // (leveragedloopv1's first real deposit-collateral attempt,
        // 2026-08-27): a real `CreateIdempotent` needed slightly more than
        // the ~4,700 CU actually left after fixed per-transaction overhead
        // ate into the requested 5,000. Never surfaced before because
        // every other real transaction bundled this with other
        // instructions carrying their own generous CU budgets, so the
        // transaction-level total always covered the shortfall regardless
        // -- only exposed once `Wallet::assemble()`'s size-based splitter
        // happened to isolate a bare ATA creation into its own
        // transaction. Bumped to a comfortable margin, not tuned to the
        // exact real minimum.
        self.append_ix(ix, 15_000);
        Some(account_id_from_pubkey(&ata_address))
    }

    /// Append an instruction. ALTs are detected automatically in
    /// assemble(). Outside an atomic group (the common case), the
    /// instruction may freely start a new transaction if `assemble()`
    /// needs to split. Inside one (see [`Self::begin_atomic_group`]),
    /// only the group's first instruction may -- every subsequent one is
    /// glued to what precedes it.
    pub fn append_ix(&mut self, ix: Instruction, compute: ComputeUnit) {
        let group_start = if self.in_atomic_group {
            let is_first = !self.atomic_group_started;
            self.atomic_group_started = true;
            is_first
        } else {
            true
        };
        self.compute += compute;
        self.q_ix.push_back((ix, compute, group_start));
    }

    /// How many instructions are currently queued -- a cheap sanity check
    /// against realistic transaction size limits (see
    /// `brain::arbv1::state::StateHelper::build_execution_plan`).
    pub fn instruction_count(&self) -> usize {
        self.q_ix.len()
    }

    /// Snapshot of the instruction queue's current length -- pair with
    /// [`Self::rollback_to`] to discard everything appended after a
    /// multi-step append (e.g. one hop of a multi-hop swap route) turns
    /// out to have failed partway through. Real, live-confirmed
    /// motivation (2026-08-27): [`Self::end_atomic_group`] alone only
    /// stops *new* instructions from being glued to the group --
    /// instructions already appended by earlier, successful steps stayed
    /// queued and were later assembled into a real transaction that
    /// landed on-chain even though the caller (`leveragedloopv1::
    /// execute_spot_leg`) believed the whole route had failed and never
    /// happened (a real $50 swapped into an unintended token as a
    /// result).
    pub fn queue_checkpoint(&self) -> usize {
        self.q_ix.len()
    }

    /// Discard every instruction appended since `checkpoint` (see
    /// [`Self::queue_checkpoint`]), restoring [`Self::cu`] to match.
    /// Safe to call whether or not an atomic group is currently open.
    pub fn rollback_to(&mut self, checkpoint: usize) {
        while self.q_ix.len() > checkpoint {
            let Some((_, compute, _)) = self.q_ix.pop_back() else {
                break;
            };
            self.compute = self.compute.saturating_sub(compute);
        }
    }

    /// Get the current compute unit for currently appended instructions.
    pub fn cu(&self) -> ComputeUnit {
        self.compute
    }

    /// Whether every instruction queued since `checkpoint` (see
    /// [`Self::queue_checkpoint`]) would fit into one signed transaction
    /// under [`Self::MAX_TX_SIZE`] -- lets a caller pre-check an atomic
    /// group's real size *before* committing to it, rather than only
    /// finding out when [`Self::assemble`] has no choice but to send it
    /// oversized (which the network then cleanly rejects). Real,
    /// live-confirmed motivation (2026-08-27): a real 4-hop USDC->jitoSOL
    /// route repeatedly built a 2219-byte atomic group (over the
    /// 1232-byte limit); `assemble()` correctly refused to send anything
    /// smaller and the network correctly rejected the oversized attempt
    /// every time, but nothing marked the route's pools as bad, so the
    /// exact same doomed route kept getting reselected on every retry.
    /// On any internal failure to probe (e.g. no blockhash available),
    /// conservatively returns `true` (assume it fits) -- `assemble()`'s
    /// own real check remains the actual safety net regardless, so a
    /// false "fits" here only costs a wasted retry, never a leak.
    pub fn atomic_group_fits(&self, checkpoint: usize) -> bool {
        if self.q_ix.len() <= checkpoint {
            return true;
        }
        let Ok(bh_raw) = transactionprocessor::blockhash() else {
            return true;
        };
        let Ok(bh_arr): Result<[u8; 32], _> = bh_raw.try_into() else {
            return true;
        };
        let blockhash = Hash::from(bh_arr);
        let Some(payer) = self.payer.as_ref().and_then(|p| self.m_key.get(p)) else {
            return true;
        };
        let payer_pubkey = rc_unlock(&payer.key).pubkey();
        let queued: Vec<(Instruction, ComputeUnit, bool)> =
            self.q_ix.iter().skip(checkpoint).cloned().collect();
        let compute: ComputeUnit = queued.iter().map(|(_, c, _)| *c).sum();
        let priority_micro_lamports = u64::from(self.priority_level);
        let l_ix = build_ix_list(&queued, compute, priority_micro_lamports);
        // Same `required_keypairs` filtering `assemble()` uses -- not
        // just defensive: real, live-confirmed 2026-09-03 that this
        // probe's own old "every registered key" `l_keypair` panicked
        // with `KeypairPubkeyMismatch` the first time this function was
        // ever exercised with two registered signer keys (the native-
        // transfer test's owner + wallet2), exactly the bug `assemble()`
        // itself hit earlier this session before this same fix. This
        // probe was never exercised with more than one registered key
        // until `send_native_transfer_via_astralane` started calling it.
        let l_keypair = Self::required_keypairs(&self.m_key, &l_ix, payer_pubkey);
        let mut probe_buf = [0u8; 4 * 1024];
        let (_, size) = build_and_serialize(
            &l_ix,
            blockhash,
            &self.l_alt,
            &l_keypair,
            payer_pubkey,
            &mut probe_buf,
        );
        size <= Self::MAX_TX_SIZE
    }

    /// Solana's real network packet-size limit
    /// (`solana_sdk::packet::PACKET_DATA_SIZE`). `self.tx_data` is
    /// deliberately much larger (4KB) so `bincode` serialization itself
    /// never fails here -- nothing else in this function checks against the
    /// real network limit, so before this constant was enforced, a
    /// too-large queue could serialize/sign/"send" cleanly (no error
    /// surfaced back to the caller) while the resulting transaction was
    /// silently dropped somewhere between the RPC and a validator, with the
    /// signature never appearing on-chain and no error ever logged. Real,
    /// live-observed: `testperpv1`'s `[11/16]` SOL borrow-hedge leg (refresh
    /// x2 + refresh_obligation + create_ata + borrow + a full CLMM swap, all
    /// queued onto one `Wallet` before a single `assemble()` drained it)
    /// repeated this exact pattern for many minutes across multiple bot
    /// restarts -- `sent transaction <sig>` logged every ~40s with zero
    /// errors, yet every signature came back "Not found" via `solana
    /// confirm` against mainnet, while the simpler 3-instruction deposit
    /// legs elsewhere in the same test consistently landed.
    const MAX_TX_SIZE: usize = 1232;

    /// Compute-unit budget for the two-instruction
    /// `create_account_with_seed` + `InitializeNonceAccount` pair (see
    /// [`Self::ensure_bundler_nonce_created`]) -- both are cheap, plain
    /// System Program instructions; not measured against a real deploy,
    /// generously sized the same way this file's other one-off
    /// account-creation budgets are (see `append_create_ata`'s own doc
    /// comment).
    const NONCE_CREATE_CU: ComputeUnit = 20_000;

    /// Compute-unit budget for a plain `system_instruction::transfer` --
    /// used for every bundler-tip transfer in this file. Real,
    /// live-confirmed bug (2026-08-28): the original value here (150)
    /// was picked as "negligible" without checking against real
    /// `ComputeBudget` instruction overhead -- confirmed via
    /// `getTransaction` on a real landed-but-failed mainnet transaction
    /// (`2VVeUzWavZyP...`, `test_send_astralane_tip_batch`'s tip
    /// transfer): `SetComputeUnitLimit`+`SetComputeUnitPrice` alone
    /// (both always prepended once `assemble()` sees a nonzero compute
    /// budget and `Wallet::priority_level`, which `finish()` sets to
    /// `Medium` after every commit) already exceed a 150 CU total
    /// budget, so instruction 1 (`SetComputeUnitPrice`) itself failed
    /// with `ComputationalBudgetExceeded` before the real transfer ever
    /// ran (`computeUnitsConsumed: 150`, exactly the limit, on-chain
    /// `err`). Same root cause as an earlier real send this session that
    /// appeared to just vanish -- it had actually landed, silently
    /// failing the same way, never independently checked at the time.
    const TRANSFER_CU: ComputeUnit = 5_000;

    /// Compute-unit budget for `advance_nonce_account` (see
    /// [`Self::send_bundler_pair`]) -- a plain System Program CPI, same
    /// order of magnitude as [`Self::TRANSFER_CU`].
    const NONCE_ADVANCE_CU: ComputeUnit = 5_000;

    /// Which of `m_key`'s registered keypairs a specific instruction batch
    /// actually needs signed by: the fee payer, plus every account any
    /// instruction in `l_ix` marks `is_signer`. Used instead of "every
    /// registered key" (the old behavior) because `Transaction::sign`/
    /// `VersionedTransaction::try_new` panic with `KeypairPubkeyMismatch`
    /// if handed a keypair that ISN'T one of the message's required
    /// signers -- harmless while a wallet only ever had one registered
    /// signer key (every deposit/withdraw/borrow/repay test so far), but
    /// real once a second one exists (the native-transfer test's two
    /// independently-signing wallets, only one of which signs any given
    /// transfer). Recomputed per candidate instruction batch, not once
    /// for the whole `assemble()` call, since the probe loop below tries
    /// several different-length prefixes of the same queue and an
    /// instruction later in the queue could need a signer none of the
    /// earlier ones do.
    fn required_keypairs<'a>(
        m_key: &'a HashMap<AccountId, SignerStatus>,
        l_ix: &[Instruction],
        payer_pubkey: Pubkey,
    ) -> Vec<&'a Keypair> {
        let mut needed: HashSet<AccountId> = HashSet::new();
        needed.insert(account_id_from_pubkey(&payer_pubkey));
        for ix in l_ix {
            for meta in &ix.accounts {
                if meta.is_signer {
                    needed.insert(account_id_from_pubkey(&meta.pubkey));
                }
            }
        }
        m_key
            .iter()
            .filter(|(id, _)| needed.contains(id))
            .map(|(_, ss)| rc_unlock(&ss.key))
            .collect()
    }

    /// Assemble and export transactions based on currently appended
    /// instructions. Scans every instruction's accounts against the loaded
    /// ALTs and automatically builds a v0 versioned transaction when any
    /// ALT account is referenced. Only takes as large a prefix of the
    /// queue as fits under [`Self::MAX_TX_SIZE`] once signed -- every
    /// caller already drains this with `while let Some(..) =
    /// wallet.assemble()`, so a queue too big for one transaction is
    /// transparently split across several calls instead of ever building
    /// one oversized transaction.
    pub fn assemble(&mut self) -> Option<(Signature, &[u8])> {
        if self.q_ix.is_empty() {
            return None;
        }
        let blockhash = {
            let bh: [u8; 32] = transactionprocessor::blockhash()
                .unwrap()
                .try_into()
                .unwrap();
            Hash::from(bh)
        };
        let priority_micro_lamports = u64::from(self.priority_level);

        let payer_pubkey =
            rc_unlock(&self.m_key.get(self.payer.as_ref().unwrap()).unwrap().key).pubkey();

        // `queued[..i]` is what a candidate transaction of the first `i`
        // queued instructions would look like; `i` grows until adding one
        // more instruction would push the signed, serialized transaction
        // past MAX_TX_SIZE -- but only ever stops at a valid group
        // boundary (`queued[i].2 == true`, i.e. not glued to what
        // precedes it -- see `begin_atomic_group`), never in the middle
        // of an atomic group. `first_group_end` is the minimum: the
        // queue's leading group can't be split further regardless of
        // size, so if it alone is oversized, send it anyway and let the
        // network reject it loudly rather than queuing it forever (same
        // philosophy the old single-instruction fallback below already
        // used, just generalized to a whole group).
        let queued: Vec<(Instruction, ComputeUnit, bool)> = self.q_ix.iter().cloned().collect();
        let first_group_end = {
            let mut k = 1;
            while k < queued.len() && !queued[k].2 {
                k += 1;
            }
            k
        };
        let mut n_ix = first_group_end;
        let mut tx_compute: ComputeUnit = queued[..n_ix].iter().map(|(_, c, _)| *c).sum();
        let mut probe_buf = [0u8; 4 * 1024];
        for i in (n_ix + 1)..=queued.len() {
            if i != queued.len() && !queued[i].2 {
                // Not a valid boundary -- growing to include queued[i]
                // would split its group. Keep extending the candidate
                // prefix without probing (size is monotonic non-decreasing
                // in prefix length, so there's nothing to test yet).
                continue;
            }
            let compute_i: ComputeUnit = queued[..i].iter().map(|(_, c, _)| *c).sum();
            let l_ix = build_ix_list(&queued[..i], compute_i, priority_micro_lamports);
            let l_keypair = Self::required_keypairs(&self.m_key, &l_ix, payer_pubkey);
            let (_, size) = build_and_serialize(
                &l_ix,
                blockhash,
                &self.l_alt,
                &l_keypair,
                payer_pubkey,
                &mut probe_buf,
            );
            if size <= Self::MAX_TX_SIZE {
                n_ix = i;
                tx_compute = compute_i;
            } else {
                break;
            }
        }

        let l_ix = build_ix_list(&queued[..n_ix], tx_compute, priority_micro_lamports);
        Self::tally_account_usage(&mut self.m_account_usage, &l_ix);
        for _ in 0..n_ix {
            let (_, compute, _) = self.q_ix.pop_front().unwrap();
            self.compute = self.compute.saturating_sub(compute);
        }
        let l_keypair = Self::required_keypairs(&self.m_key, &l_ix, payer_pubkey);
        let (signature, size) = build_and_serialize(
            &l_ix,
            blockhash,
            &self.l_alt,
            &l_keypair,
            payer_pubkey,
            &mut self.tx_data[0..],
        );
        if Self::MAX_TX_SIZE < size {
            // n_ix is only ever grown past first_group_end once the probe
            // loop confirms a larger, still-boundary-valid prefix fits, so
            // a still-oversized final build means n_ix == first_group_end
            // -- the queue's leading atomic group, unsplittable, exceeds
            // the limit on its own.
            log_error!(
                "wallet: assemble() -- leading instruction group ({n_ix} ix, program_id={}) exceeds {}-byte tx size limit ({size} bytes), sending oversized anyway",
                queued[0].0.program_id,
                Self::MAX_TX_SIZE,
            );
        }
        Some((signature, &self.tx_data[0..size]))
    }

    /// Drains every transaction [`Self::assemble`] produces this call and
    /// sends them all via the host's `batch` import (`count: list<u16>,
    /// txdata: list<u8>, bundler: option<u8>` in `wit/component.wit`),
    /// tagged `crate::bundler_config::BUNDLER` -- **always**, including a
    /// single (or zero) resulting transaction. Added 2026-08-28 per
    /// explicit instruction: every bot mode's `evaluate()` used to drain
    /// with `while let Some((sig, data)) = wallet.assemble() { send(...) }`,
    /// sending each transaction its own untagged host round-trip via the
    /// bundler-blind `send` import. Changed again the same day, also per
    /// explicit instruction ("route all transactions through batch
    /// bundle=astralane, including single transactions") -- this used to
    /// special-case a single (or zero) resulting transaction back to
    /// `send`, which skipped the `bundler` tag entirely for what turned
    /// out to be most of this bot's real sends (single-transaction ticks
    /// are the common case). Real bundler identity (Jito vs Astralane,
    /// etc.) is a host-side concern this function never interprets, just
    /// plumbs through. Returns each transaction's signature paired with
    /// its own send-time result, in the same order `assemble()` produced
    /// them, so the caller can log with its own module-specific message
    /// prefix -- `batch`'s single whole-call result (the WIT contract has
    /// no per-transaction signal) is applied to every transaction in that
    /// batch alike, same "all-or-nothing" semantics the old per-transaction
    /// `send` calls already had individually.
    ///
    /// Does **not** attach a bundler tip to the *real* transaction(s) --
    /// a real Astralane race-pair needs a specific two-transaction,
    /// same-durable-nonce, fee-variant pair with the tip baked into each
    /// variant's own instructions (see [`Self::send_bundler_pair`]), not
    /// a tip appended to whatever this tick's unrelated transactions
    /// happen to be.
    ///
    /// **Real, live-confirmed dead end (2026-08-28, tried and reverted
    /// twice today):** an earlier version of this function auto-appended
    /// a tip instruction to arbitrary queued instructions -- unsafe,
    /// since `send_ideal` races whatever two transactions it's given and
    /// cancels the loser via a shared nonce, so tagging an arbitrary
    /// N-transaction batch risked canceling an unrelated, necessary
    /// transaction that lost the race. The next attempt built a separate
    /// *companion* transaction that paid the tip on its own, paired
    /// alongside the untouched real transaction, purely to satisfy
    /// catscope-zerohop's `TipRoutingHandler` exactly-2-transaction count
    /// requirement for `bundler=astralane`. Real, live-confirmed result:
    /// the companion landed (it paid a real tip), the real transaction
    /// never did (it paid none) -- not a race at all, just two fully
    /// independent transactions with independently determined landing
    /// odds. A tip only helps the transaction it's actually attached to;
    /// pairing an untipped real transaction next to a tipped throwaway
    /// one does nothing for the real transaction and just burns the tip
    /// lamports for no benefit. Getting a real transaction real
    /// Astralane-prioritized landing requires the tip to be an
    /// instruction *inside that same transaction* -- which, to safely
    /// still satisfy the exactly-2 count without risking a double
    /// execution if both variants landed, requires the same-durable-nonce
    /// mutual-exclusion `send_bundler_pair` already provides.
    ///
    /// So (2026-08-28, explicit: "If the priority is set to high in
    /// Wallet::set_priority_fee(), then set the landing priority to
    /// High"): whenever [`Self::desired_landing_high`] is set (mirrors
    /// the caller's last `set_priority_fee(PriorityLevel::High)` call --
    /// see that method's doc comment) and this build is Astralane-
    /// configured, this tries [`Self::send_bundler_pair`] *first*, before
    /// touching the queue via `assemble()` at all. `send_bundler_pair`
    /// only ever drains the queue after every precondition (payer set,
    /// durable nonce `Ready`, a live tip recommendation) already passed
    /// (see its own doc comment), so a decline here is always safe to
    /// fall through from -- the queue is exactly as the caller left it,
    /// and the ordinary path below picks it up and sends it via the
    /// plain default route, same as if `desired_landing_high` had never
    /// been set. This is deliberately best-effort, not "block until the
    /// nonce is ready" -- a caller wanting a hard guarantee should call
    /// `send_bundler_pair` directly instead (see its own doc comment for
    /// the checkpoint/rollback discipline that requires).
    pub fn drain_and_send(
        &mut self,
    ) -> Vec<(Signature, Result<(), transactionprocessor::ErrorCode>)> {
        if self.desired_landing_high
            && crate::bundler_config::BUNDLER == Self::ASTRALANE_BUNDLER_CODE
        {
            if let Some((sig, result)) = self.send_bundler_pair(crate::bundler_config::BUNDLER) {
                return vec![(sig, result)];
            }
        }
        let mut txs: Vec<(Signature, Vec<u8>)> = Vec::new();
        while let Some((sig, data)) = self.assemble() {
            txs.push((sig, data.to_vec()));
        }
        if txs.is_empty() {
            return Vec::new();
        }
        let bundler = if crate::bundler_config::BUNDLER == Self::ASTRALANE_BUNDLER_CODE {
            None
        } else {
            Some(crate::bundler_config::BUNDLER)
        };
        let mut l_txdata: Vec<u8> = Vec::new();
        let mut l_count: Vec<u16> = Vec::with_capacity(txs.len());
        for (_sig, data) in &txs {
            l_txdata.extend_from_slice(data);
            l_count.push(data.len() as u16);
        }
        log_warn!(
            "wallet: drain_and_send batching {} transaction(s) via transactionprocessor::batch (bundler={:?})",
            txs.len(),
            bundler,
        );
        let result = transactionprocessor::batch(&l_count, &l_txdata, bundler);
        txs.into_iter().map(|(sig, _)| (sig, result)).collect()
    }

    /// Tallies every non-signer, non-well-known-program account referenced
    /// across `l_ix` into `m_account_usage`. Called once per
    /// [`Self::assemble`] call with that call's finalized instruction list
    /// -- the same account-meta shape [`build_and_serialize`] already
    /// walks to find ALT matches, just counting instead of matching.
    /// Takes the map directly (not `&mut self`) so the call site can
    /// borrow just this one field -- `assemble()` already holds an
    /// immutable borrow of `self.m_key` (via `l_keypair`) at the point
    /// this needs to run.
    fn tally_account_usage(
        m_account_usage: &mut HashMap<AccountId, (Pubkey, u32), BuildHasherDefault<XxHash64>>,
        l_ix: &[Instruction],
    ) {
        for ix in l_ix {
            for meta in &ix.accounts {
                if meta.is_signer || ALT_EXCLUDED_PROGRAMS.contains(&meta.pubkey) {
                    continue;
                }
                let account_id = account_id_from_pubkey(&meta.pubkey);
                let entry = m_account_usage
                    .entry(account_id)
                    .or_insert((meta.pubkey, 0));
                entry.1 += 1;
            }
        }
    }

    /// Returns up to `n` of the most-referenced non-signer accounts this
    /// wallet has ever assembled into a transaction, ranked by usage count
    /// descending (pubkey-bytes ascending as a deterministic tiebreak).
    /// Feeds `MessageSend::CommonAddressUpdate`.
    pub fn top_account_usage(&self, n: usize) -> Vec<(Pubkey, u32)> {
        let mut l: Vec<(Pubkey, u32)> = self.m_account_usage.values().copied().collect();
        l.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| a.0.to_bytes().cmp(&b.0.to_bytes()))
        });
        l.truncate(n);
        l
    }

    /// Apply a live bundler tip update pushed by the optimizer (see
    /// `bundler_message::BundlerTipUpdate`/
    /// `COMMON_KEY_FLAG_BUNDLER_TIP_UPDATE`, and
    /// `optimizer/bundler.RunTipBroadcaster`, the Go-side poller that
    /// sends these). A "down" update (`update.up == false`, sent
    /// whenever Go's `Bundler.Tip()`/`Distribution()` returned an error)
    /// clears this bundler's address list so `select_tip_account`/
    /// `append_bundler_tip` return nothing for it rather than risk
    /// tipping a stale/unreachable account. Subscribes to any tip
    /// address not already known (see `hs_known_tip_accounts`) so
    /// `pubkey_from_account_id` never has to round-trip the host the
    /// first time a tip instruction actually references one -- same
    /// "subscribe before you might reference it in an instruction"
    /// discipline this wallet's ATA/Kamino-position subscriptions
    /// already follow.
    pub fn apply_bundler_tip_update(&mut self, graph: &Graph, update: BundlerTipUpdate) {
        if !update.up || update.tip_addresses.is_empty() {
            self.m_bundler_tip
                .insert(update.bundler, BundlerTipState::default());
            return;
        }
        let mut reqs = Vec::new();
        let mut tip_account_ids = Vec::with_capacity(update.tip_addresses.len());
        for pubkey in &update.tip_addresses {
            let account_id = account_id_from_pubkey(pubkey);
            tip_account_ids.push(account_id);
            if self.hs_known_tip_accounts.insert(account_id) {
                reqs.push(SubscriptionRequest {
                    root: account_id,
                    filter_weight: 0,
                    depth: 1,
                });
            }
        }
        if !reqs.is_empty() {
            match SubscriptionQueue::subscribe_now(graph, reqs) {
                Ok(subs) => self.l_tip_sub.extend(subs),
                Err(e) => log_error!(
                    "wallet: failed to subscribe to new bundler {} tip accounts: {e}",
                    update.bundler
                ),
            }
        }
        self.m_bundler_tip.insert(
            update.bundler,
            BundlerTipState {
                up: true,
                tip_account_ids,
                distribution: update.distribution,
            },
        );
    }

    /// The known tip account for `bundler` this wallet's own
    /// transactions have referenced least often so far
    /// (`m_account_usage`'s existing tally -- a tip transfer instruction
    /// gets counted there like any other account reference once queued),
    /// to spread tips across the bundler's address list instead of
    /// hammering the same one every time and contending for its write
    /// lock against every other bot doing the same. `None` if `bundler`
    /// is down or has no known tip accounts yet.
    pub fn select_tip_account(&self, bundler: u8) -> Option<AccountId> {
        let state = self.m_bundler_tip.get(&bundler)?;
        if !state.up {
            return None;
        }
        state.tip_account_ids.iter().copied().min_by_key(|id| {
            self.m_account_usage
                .get(id)
                .map(|(_, count)| *count)
                .unwrap_or(0)
        })
    }

    /// Whether `bundler` has ever reported real status at all, up or
    /// down -- distinct from [`Self::select_tip_account`] returning
    /// `None`, which is also (indistinguishably, from that call alone)
    /// what "no status has arrived yet" looks like. Exists for callers
    /// that need to tell those two apart: real, live-verified 2026-09-04
    /// that sending a transaction before the bundler tip broadcaster's
    /// first successful update ever reaches this guest (its own very
    /// first attempt always fails with "bot not connected yet" -- an
    /// inherent chicken/egg at boot, see `bundler.RunTipBroadcaster`'s
    /// own poll()) makes that transaction fall back to a slower, non-
    /// bundled send it didn't need to -- purely an artifact of racing
    /// ahead of data that was always going to arrive shortly after.
    pub fn has_bundler_status(&self, bundler: u8) -> bool {
        self.m_bundler_tip.contains_key(&bundler)
    }

    /// Real Astralane tip-distribution snapshot, captured live
    /// (`{"time": "2026-04-22T11:22:52Z", "landed_tips_25th_percentile":
    /// 10000, "landed_tips_50th_percentile": 21000,
    /// "landed_tips_75th_percentile": 77952, "landed_tips_95th_percentile":
    /// 1000000, "landed_tips_99th_percentile": 4759970}`) before
    /// Astralane's tip-stream/landing service degraded to consistently
    /// all-zero (see [`Self::tip_lamports`]'s doc comment) -- explicit
    /// default (2026-08-28) used only for `ASTRALANE_BUNDLER_CODE`
    /// whenever the *live* distribution computes to 0, so a real,
    /// plausible tip still gets paid instead of declining Astralane
    /// routing outright while their feed stays degraded.
    const ASTRALANE_DEFAULT_DISTRIBUTION: [u64; 5] = [10_000, 21_000, 77_952, 1_000_000, 4_759_970];

    /// Recommended tip size for `bundler`, in lamports -- the live
    /// distribution's 95th percentile (`High` landing probability --
    /// pays more than `Medium`'s 75th percentile to further reduce the
    /// chance of a slow/missed landing). `None` if `bundler` is down or
    /// no distribution has been received yet. If the live distribution
    /// is degenerate (computes to 0 lamports -- real, live-confirmed
    /// 2026-08-28: a `CommonBundlerTipUpdate` marked `up` but carrying an
    /// all-zero distribution, matching Astralane's own tip-stream
    /// returning an all-zero snapshot -- Astralane's docs say the stream
    /// reflects their own recent successful landings, so an all-zero
    /// snapshot means ~nothing is landing through them right now, not a
    /// client bug) and `bundler` is Astralane specifically, falls back to
    /// [`Self::ASTRALANE_DEFAULT_DISTRIBUTION`] rather than declining --
    /// for any other bundler, still declines (`None`). Every caller gets
    /// this automatically, rather than each re-checking for zero itself.
    /// Routes through `trader::bundler::AstralaneTips::landing` (rather
    /// than indexing `state.distribution` directly) so the desired
    /// landing probability is one visible, named choice instead of a
    /// magic array index.
    pub fn tip_lamports(&self, bundler: u8) -> Option<u64> {
        let state = self.m_bundler_tip.get(&bundler)?;
        if !state.up {
            return None;
        }
        let tips = crate::trader::bundler::AstralaneTips {
            landed_tips: state.distribution,
        };
        let lamports = tips.landing(crate::trader::bundler::LandingProbability::High);
        if lamports == 0 {
            if bundler == Self::ASTRALANE_BUNDLER_CODE {
                let default_tips = crate::trader::bundler::AstralaneTips {
                    landed_tips: Self::ASTRALANE_DEFAULT_DISTRIBUTION,
                };
                return Some(
                    default_tips.landing(crate::trader::bundler::LandingProbability::High),
                );
            }
            return None;
        }
        Some(lamports)
    }

    /// Appends a `system_instruction::transfer` tip from `owner` to
    /// `bundler`'s current least-used known tip account (see
    /// [`Self::select_tip_account`]), sized at [`Self::tip_lamports`]'s
    /// live `High`-landing-probability recommendation. Returns `false`
    /// (appending nothing) if `bundler` is down, has no known tip
    /// accounts, or no live distribution has arrived yet -- **no
    /// hardcoded fallback amount** (removed 2026-08-28, real,
    /// live-confirmed: a fixed `MIN_BUNDLER_TIP_LAMPORTS` floor silently
    /// paid a stale 10,000-lamport tip -- the real p25, not `High`/p95 --
    /// whenever live distribution data was degenerate, with no signal
    /// that anything was wrong). Callers should treat a `false` return as
    /// "no trustworthy tip size yet, send without a bundler tip" rather
    /// than block on it, same as every other best-effort wallet helper in
    /// this file.
    pub fn append_bundler_tip(&mut self, owner: AccountId, bundler: u8) -> bool {
        let Some(tip_account_id) = self.select_tip_account(bundler) else {
            return false;
        };
        let Some(lamports) = self.tip_lamports(bundler) else {
            return false;
        };
        let (Some(owner_pubkey), Some(tip_pubkey)) = (
            pubkey_from_account_id(&owner),
            pubkey_from_account_id(&tip_account_id),
        ) else {
            return false;
        };
        self.append_ix(
            solana_system_interface::instruction::transfer(&owner_pubkey, &tip_pubkey, lamports),
            Self::TRANSFER_CU,
        );
        true
    }

    /// A real dual-transaction Astralane bundle: two transactions sharing
    /// the same durable nonce (see [`Self::ensure_bundler_nonce_created`]/
    /// [`Self::nonce_ready`]) -- one heavy-tip/no-priority-fee, one
    /// small-tip/heavy-priority-fee -- built from whatever is currently
    /// queued (drained by this call, same as [`Self::drain_and_send`]),
    /// submitted in one `transactionprocessor::batch` call. Astralane
    /// races the two and cancels the loser via the shared nonce (see
    /// `catscope-zerohop::astralane::AstralaneHandler::send_ideal`'s doc
    /// comment). [`Self::drain_and_send`] tries this first, automatically,
    /// whenever the caller asked for `PriorityLevel::High` (see
    /// [`Self::set_priority_fee`]'s doc comment) and this build is
    /// Astralane-configured -- that's the intended way for ordinary
    /// `evaluate()` code to opt a real operation into Astralane landing;
    /// calling this directly (as the `TriggerTestBundler` test trigger
    /// does) remains for one-shot/manual use.
    ///
    /// Callers are responsible for keeping whatever's queued small/
    /// single-purpose before calling this -- neither variant goes through
    /// [`Self::assemble`]'s size-driven splitter, since splitting would
    /// break the "one pair, one intent" contract `send_ideal` needs.
    ///
    /// Returns `None` on a precondition miss (no payer set, the
    /// durable-nonce account isn't confirmed `Ready` yet -- see
    /// [`Self::ensure_bundler_nonce_created`] -- nothing is queued, or
    /// `bundler` has no known tip account) *without draining the queue*
    /// -- whatever the caller already appended before calling this is
    /// still sitting there afterward, untouched. `drain_and_send` relies
    /// on exactly this to fall through safely to its own ordinary path
    /// on a decline. A **manual/standalone** caller that wants a strict
    /// bundler-or-nothing outcome (not `drain_and_send`'s automatic
    /// integration) must instead `queue_checkpoint()` before appending
    /// and `rollback_to()` on any non-`Some(Ok(_))` result (mirrors
    /// `execute_spot_leg`'s own checkpoint/rollback discipline around its
    /// own multi-step append) -- `send_bundler_pair` itself can't do this
    /// safely, since it has no way to distinguish "instructions meant
    /// only for this bundle" from anything else already queued by
    /// unrelated code this same tick. Real, live-confirmed gap
    /// (2026-08-28) that motivated this: a real `TriggerTestBundler` test
    /// send skipped the checkpoint once and leaked a stray instruction
    /// into an ordinary send.
    ///
    /// Returns the heavy-tip variant's own signature alongside the send
    /// result -- both variants carry the same real instructions and only
    /// one can ever land (the durable nonce guarantees that), so this is
    /// simply "the" signature to report back to callers that need one
    /// (e.g. `drain_and_send`'s return contract); variant B's signature
    /// is still logged separately below for independent verification.
    pub fn send_bundler_pair(
        &mut self,
        bundler: u8,
    ) -> Option<(Signature, Result<(), transactionprocessor::ErrorCode>)> {
        debug_assert!(!self.in_atomic_group, "send_bundler_pair mid atomic-group");
        if self.q_ix.is_empty() {
            return None;
        }
        let payer = self.payer?;
        let nonce_blockhash = self.nonce_ready()?;
        let owner_pubkey = pubkey_from_account_id(&payer)?;
        let tip_account_id = self.select_tip_account(bundler)?;
        let tip_pubkey = pubkey_from_account_id(&tip_account_id)?;
        let nonce_pubkey = Self::nonce_address(&owner_pubkey);
        let advance_nonce_ix = solana_system_interface::instruction::advance_nonce_account(
            &nonce_pubkey,
            &owner_pubkey,
        );

        let queued: Vec<(Instruction, ComputeUnit, bool)> = self.q_ix.drain(..).collect();
        let real_ix: Vec<Instruction> = queued.iter().map(|(ix, _, _)| ix.clone()).collect();
        let real_compute: ComputeUnit = queued.iter().map(|(_, c, _)| *c).sum();
        let combined_compute = real_compute + Self::NONCE_ADVANCE_CU + Self::TRANSFER_CU;

        // No hardcoded fallback amount (removed 2026-08-28, real,
        // live-confirmed: a fixed MIN_BUNDLER_TIP_LAMPORTS floor silently
        // paid a stale 10,000-lamport tip -- the real p25, not
        // `High`/p95 -- whenever live distribution data was degenerate,
        // with no signal anything was wrong) -- both variants pay
        // `tip_lamports`'s live `High`-probability recommendation;
        // declines (via `?`) if none has arrived yet rather than
        // guessing.
        let tip = self.tip_lamports(bundler)?;
        let l_ix_a = build_bundler_variant(
            advance_nonce_ix.clone(),
            solana_system_interface::instruction::transfer(&owner_pubkey, &tip_pubkey, tip),
            &real_ix,
            combined_compute,
            0, // the tip itself is the incentive -- no extra priority fee
        );
        let l_ix_b = build_bundler_variant(
            advance_nonce_ix,
            solana_system_interface::instruction::transfer(&owner_pubkey, &tip_pubkey, tip),
            &real_ix,
            combined_compute,
            u64::from(PriorityLevel::High),
        );

        let mut l_keypair = Vec::with_capacity(self.m_key.len());
        for (_, ss) in self.m_key.iter() {
            l_keypair.push(rc_unlock(&ss.key));
        }

        let mut buf_a = [0u8; 4 * 1024];
        let mut buf_b = [0u8; 4 * 1024];
        let (sig_a, size_a) = build_and_serialize(
            &l_ix_a,
            nonce_blockhash,
            &self.l_alt,
            &l_keypair,
            owner_pubkey,
            &mut buf_a,
        );
        let (sig_b, size_b) = build_and_serialize(
            &l_ix_b,
            nonce_blockhash,
            &self.l_alt,
            &l_keypair,
            owner_pubkey,
            &mut buf_b,
        );
        if Self::MAX_TX_SIZE < size_a || Self::MAX_TX_SIZE < size_b {
            // Real, live-confirmed gap (2026-09-05): this used to return
            // `Some((sig_a, Err(InvalidArgument)))` -- a real, terminal
            // failure result, per this function's own doc comment ("`Some`
            // on precondition miss" is reserved for declines the caller
            // can safely fall through from; this wasn't one). Worse, by
            // this point `self.q_ix` had already been drained into
            // `queued` above, so the real underlying instructions were
            // simply gone -- a route that would have fit fine as an
            // ordinary, un-tipped transaction (this pair's own overhead,
            // not the real trade, is what pushed it over the limit) failed
            // outright with nothing left to fall back to. Confirmed live:
            // a real 2-hop close, `desired_landing_high` now set for
            // every spot leg (see `execute_spot_leg`'s own doc comment),
            // hit exactly this. Restore the queue and decline via `None`
            // instead -- same "caller's ordinary path picks it up
            // untouched" contract every other precondition-miss case here
            // already uses.
            log_error!(
                "wallet: send_bundler_pair -- variant sizes ({size_a}, {size_b}) exceed {}-byte tx size limit -- declining, ordinary path should pick this up instead",
                Self::MAX_TX_SIZE,
            );
            self.q_ix.extend(queued);
            return None;
        }

        Self::tally_account_usage(&mut self.m_account_usage, &l_ix_a);
        Self::tally_account_usage(&mut self.m_account_usage, &l_ix_b);

        let mut txdata = Vec::with_capacity(size_a + size_b);
        txdata.extend_from_slice(&buf_a[..size_a]);
        txdata.extend_from_slice(&buf_b[..size_b]);
        let l_count = [size_a as u16, size_b as u16];
        // Real, live-confirmed gap (2026-08-28): this used to discard
        // both signatures (`build_and_serialize` was called as `let (_,
        // size) = ...`), so a real send here could only ever be verified
        // indirectly (via the nonce/tip account's tx history), not
        // directly by signature -- logged explicitly now so both legs of
        // a real durable-nonce race are independently checkable
        // (`getSignatureStatuses`), same discipline every other real
        // send this session already follows.
        log_warn!(
            "wallet: send_bundler_pair -- sending durable-nonce pair via bundler {bundler}: variant A (heavy-tip) {sig_a} ({size_a}B, tip={tip}); variant B (heavy-fee) {sig_b} ({size_b}B, tip={tip})",
        );
        Some((
            sig_a,
            transactionprocessor::batch(&l_count, &txdata, Some(bundler)),
        ))
    }

    /// Sends `transactions` (each already independently built via its own
    /// [`Self::assemble`] call) together as one real
    /// `transactionprocessor::batch` call, always with `Some(bundler)` --
    /// for a real, atomically-landed multi-transaction bundle, not
    /// [`Self::drain_and_send`]'s own ordinary multi-tx path (which passes
    /// `None` whenever the configured bundler is Astralane, specifically
    /// to *avoid* double-bundling through it there -- see that function's
    /// own body). Real, live-confirmed real-world use case this exists
    /// for: a single swap route whose combined instructions don't fit in
    /// one 1232-byte transaction (`Self::atomic_group_fits` failing) can
    /// still land as one atomic unit by building each hop as its own
    /// separate, individually-tipped transaction and sending them all
    /// here together, instead of failing outright -- see
    /// `testperpv1::state::StateHelper::execute_spot_leg`'s own
    /// oversized-route fallback. Same real host primitive already proven by
    /// [`Self::send_bundler_pair`]/[`Self::test_send_two_system_transfers`],
    /// generalized from a fixed pair to an arbitrary N.
    ///
    /// Callers are responsible for having appended a real bundler tip
    /// ([`Self::append_bundler_tip`]) to *every* transaction before
    /// assembling it -- confirmed 2026-08-28: Astralane requires a tip on
    /// each transaction it's asked to route, not just one.
    pub fn send_transaction_batch(
        &self,
        transactions: &[Vec<u8>],
        bundler: u8,
    ) -> Result<(), transactionprocessor::ErrorCode> {
        if transactions.is_empty() || transactions.len() > u16::MAX as usize {
            return Err(transactionprocessor::ErrorCode::InvalidArgument);
        }
        let l_count: Vec<u16> = transactions.iter().map(|t| t.len() as u16).collect();
        let mut l_txdata = Vec::with_capacity(transactions.iter().map(Vec::len).sum());
        for t in transactions {
            l_txdata.extend_from_slice(t);
        }
        transactionprocessor::batch(&l_count, &l_txdata, Some(bundler))
    }

    /// Test-only helper: forces the `transactionprocessor::batch` host
    /// import (the generic N-transaction path `drain_and_send` uses, not
    /// `send_bundler_pair`'s durable-nonce pair) with two deliberately
    /// inert self-transfer transactions. `assemble()` greedily packs
    /// everything currently queued into as few transactions as fit, so
    /// two small system transfers queued together would land in a single
    /// transaction rather than two -- getting genuinely separate
    /// transactions out of it means queuing and assembling them one at a
    /// time instead. Requires the queue to already be empty (so this call
    /// owns the whole thing and neither self-transfer picks up an
    /// unrelated instruction some other code queued this tick); returns
    /// `None` without sending anything if it isn't, or if `owner`'s key
    /// isn't loaded yet.
    ///
    /// Real requirement (2026-08-28, explicit): Astralane requires **every**
    /// transaction it's asked to route to pay its own tip -- not just one
    /// of the two, regardless of position. So whenever `bundler` is
    /// `Some(_)`, both transactions get a tip transfer to
    /// [`Self::ASTRALANE_TIP_ACCOUNT`] prepended ahead of their self-transfer
    /// (address hardcoded rather than drawn from `select_tip_account`,
    /// same reasoning as [`Self::test_send_astralane_tip_batch`] -- this
    /// is the only bundler this generic helper knows how to tip today; a
    /// real non-Astralane bundler passed here would still send both legs
    /// untipped). The tip **amount** has no hardcoded fallback (removed
    /// 2026-08-28, see [`Self::send_bundler_pair`]'s doc comment) -- both
    /// legs pay [`Self::tip_lamports`]'s live `High`-probability
    /// recommendation; this whole call declines (returns `None`) if none
    /// has arrived yet.
    pub fn test_send_two_system_transfers(
        &mut self,
        owner: AccountId,
        bundler: Option<u8>,
    ) -> Option<Result<(), transactionprocessor::ErrorCode>> {
        if !self.q_ix.is_empty() {
            return None;
        }
        let owner_pubkey = pubkey_from_account_id(&owner)?;
        let tip = match bundler {
            Some(_) => Some(self.tip_lamports(Self::ASTRALANE_BUNDLER_CODE)?),
            None => None,
        };
        self.require_signer(owner);

        if let Some(tip) = tip {
            self.append_ix(
                solana_system_interface::instruction::transfer(
                    &owner_pubkey,
                    &Self::ASTRALANE_TIP_ACCOUNT,
                    tip,
                ),
                Self::TRANSFER_CU,
            );
        }
        self.append_ix(
            solana_system_interface::instruction::transfer(&owner_pubkey, &owner_pubkey, 5_000),
            Self::TRANSFER_CU,
        );
        let (_, data1) = self.assemble()?;
        let tx1 = data1.to_vec();

        if let Some(tip) = tip {
            self.append_ix(
                solana_system_interface::instruction::transfer(
                    &owner_pubkey,
                    &Self::ASTRALANE_TIP_ACCOUNT,
                    tip,
                ),
                Self::TRANSFER_CU,
            );
        }
        self.append_ix(
            solana_system_interface::instruction::transfer(&owner_pubkey, &owner_pubkey, 6_000),
            Self::TRANSFER_CU,
        );
        let (_, data2) = self.assemble()?;
        let tx2 = data2.to_vec();

        let l_count = [tx1.len() as u16, tx2.len() as u16];
        let mut l_txdata = Vec::with_capacity(tx1.len() + tx2.len());
        l_txdata.extend_from_slice(&tx1);
        l_txdata.extend_from_slice(&tx2);
        log_warn!(
            "wallet: test_send_two_system_transfers -- sending 2 separate transactions ({}B, {}B) via transactionprocessor::batch (bundler={bundler:?})",
            tx1.len(),
            tx2.len(),
        );
        Some(transactionprocessor::batch(&l_count, &l_txdata, bundler))
    }

    /// Astralane's own registered bundler tag -- see `Code()` in
    /// `optimizer/bundler/astralane/astralane.go` (`bundler.BundlerAstralane`,
    /// `optimizer/bundler/bundler.go`). Same value `TxBundler::Astralane`
    /// tags on the host side (`catscope-zerohop`/`catscope-geyser`).
    pub(crate) const ASTRALANE_BUNDLER_CODE: u8 = 1;

    /// One of Astralane's real tip wallets -- `optimizer/bundler/astralane`'s
    /// `Tip()` returns the full round-robin list (`listTipAddress`) this is
    /// drawn from; this is `listTipAddress[1]`, the same address
    /// `catscope_zerohop::astralane::ASTRALANE_TIP_ACCOUNT` already uses
    /// host-side. Hardcoded rather than drawn from `select_tip_account`
    /// (which needs a live `CommonBundlerTipUpdate` broadcast from
    /// `optimizer/bundler.RunTipBroadcaster` to have already arrived) so
    /// this test works the moment a wallet key is loaded.
    const ASTRALANE_TIP_ACCOUNT: Pubkey =
        solana_sdk::pubkey!("astra4uejePWneqNaJKuFFA8oonqCE1sqF6b45kDMZm");

    /// Test-only helper: forces `transactionprocessor::batch` routed to
    /// Astralane specifically (`ASTRALANE_BUNDLER_CODE`) -- both
    /// transactions pay a real tip to `ASTRALANE_TIP_ACCOUNT` (Astralane
    /// requires a tip on every transaction it routes, not just one of the
    /// two, regardless of position -- confirmed explicitly 2026-08-28),
    /// each paired with its own deliberately inert self-transfer. See
    /// [`Self::test_send_two_system_transfers`]'s doc comment for why two
    /// *separate* transactions requires assembling them one at a time
    /// rather than queuing both up front. The tip amount has no
    /// hardcoded fallback (removed 2026-08-28, see
    /// [`Self::send_bundler_pair`]'s doc comment) -- both legs pay
    /// [`Self::tip_lamports`]'s live `High`-probability recommendation;
    /// declines (returns `None`) if none has arrived yet.
    pub fn test_send_astralane_tip_batch(
        &mut self,
        owner: AccountId,
    ) -> Option<Result<(), transactionprocessor::ErrorCode>> {
        if !self.q_ix.is_empty() {
            return None;
        }
        let owner_pubkey = pubkey_from_account_id(&owner)?;
        let tip = self.tip_lamports(Self::ASTRALANE_BUNDLER_CODE)?;
        self.require_signer(owner);

        self.append_ix(
            solana_system_interface::instruction::transfer(
                &owner_pubkey,
                &Self::ASTRALANE_TIP_ACCOUNT,
                tip,
            ),
            Self::TRANSFER_CU,
        );
        self.append_ix(
            solana_system_interface::instruction::transfer(&owner_pubkey, &owner_pubkey, 5_000),
            Self::TRANSFER_CU,
        );
        let (_, data1) = self.assemble()?;
        let tx1 = data1.to_vec();

        self.append_ix(
            solana_system_interface::instruction::transfer(
                &owner_pubkey,
                &Self::ASTRALANE_TIP_ACCOUNT,
                tip,
            ),
            Self::TRANSFER_CU,
        );
        self.append_ix(
            solana_system_interface::instruction::transfer(&owner_pubkey, &owner_pubkey, 6_000),
            Self::TRANSFER_CU,
        );
        let (_, data2) = self.assemble()?;
        let tx2 = data2.to_vec();

        let l_count = [tx1.len() as u16, tx2.len() as u16];
        let mut l_txdata = Vec::with_capacity(tx1.len() + tx2.len());
        l_txdata.extend_from_slice(&tx1);
        l_txdata.extend_from_slice(&tx2);
        log_warn!(
            "wallet: test_send_astralane_tip_batch -- sending 2 transactions, each tipping {tip} lamports to {} ({}B, {}B), via transactionprocessor::batch (bundler=astralane)",
            Self::ASTRALANE_TIP_ACCOUNT,
            tx1.len(),
            tx2.len(),
        );
        Some(transactionprocessor::batch(
            &l_count,
            &l_txdata,
            Some(Self::ASTRALANE_BUNDLER_CODE),
        ))
    }
}

/// Compute-budget + priority-fee instructions (if any), followed by
/// `ixs`'s instructions in order -- the exact prefix every assembled
/// transaction needs, factored out so [`Wallet::assemble`]'s size-probing
/// loop and its final build use identical logic.
fn build_ix_list(
    ixs: &[(Instruction, ComputeUnit, bool)],
    compute: ComputeUnit,
    priority_micro_lamports: u64,
) -> Vec<Instruction> {
    let mut l_ix = Vec::with_capacity(ixs.len() + 2);
    if 0 < compute {
        l_ix.push(ComputeBudgetInstruction::set_compute_unit_limit(compute));
    }
    if 0 < priority_micro_lamports {
        l_ix.push(ComputeBudgetInstruction::set_compute_unit_price(
            priority_micro_lamports,
        ));
    }
    for (ix, _, _) in ixs {
        l_ix.push(ix.clone());
    }
    l_ix
}

/// One [`Wallet::send_bundler_pair`] variant's full instruction list:
/// compute-budget + priority-fee instructions (if any), `advance_nonce_ix`,
/// `tip_ix`, then `real_ix` in order -- the shared shape both of
/// `send_bundler_pair`'s variants build, differing only in `tip_ix`'s
/// amount and `priority_micro_lamports`.
fn build_bundler_variant(
    advance_nonce_ix: Instruction,
    tip_ix: Instruction,
    real_ix: &[Instruction],
    compute: ComputeUnit,
    priority_micro_lamports: u64,
) -> Vec<Instruction> {
    // Real, live-confirmed bug (2026-08-28): a durable-nonce transaction
    // is only recognized as one by the network if `AdvanceNonceAccount`
    // is literally the *first* instruction (index 0) -- a hard Solana
    // protocol requirement, not just a convention. This function used to
    // put the compute-budget instructions first instead, so every real
    // `send_bundler_pair` transaction (2-for-2 failed attempts) looked
    // like an ordinary transaction with a stale, unrecognized
    // `recent_blockhash` to the network -- confirmed via
    // `simulateTransaction` on the exact bytes this used to produce:
    // `err: "BlockhashNotFound"`, the same symptom a genuinely expired
    // blockhash would give, even though the nonce account's real stored
    // value was used. `AdvanceNonceAccount` must come before *any* other
    // instruction, including `ComputeBudgetInstruction`s.
    let mut l_ix = Vec::with_capacity(4 + real_ix.len());
    l_ix.push(advance_nonce_ix);
    if 0 < compute {
        l_ix.push(ComputeBudgetInstruction::set_compute_unit_limit(compute));
    }
    if 0 < priority_micro_lamports {
        l_ix.push(ComputeBudgetInstruction::set_compute_unit_price(
            priority_micro_lamports,
        ));
    }
    l_ix.push(tip_ix);
    l_ix.extend_from_slice(real_ix);
    l_ix
}

/// Sign `l_ix` (against an ALT-aware v0 message if any instruction touches
/// a loaded ALT, otherwise a legacy message) and serialize into `buf`.
/// Factored out of [`Wallet::assemble`] so its size-probing loop can try
/// candidate instruction prefixes against a throwaway buffer without
/// touching `self.tx_data` (which only the winning, final candidate
/// writes into) -- takes plain borrowed pieces instead of `&self` since a
/// `&self`-taking method can't be called while `&mut self.tx_data` is
/// simultaneously borrowed for the same call's argument.
fn build_and_serialize(
    l_ix: &[Instruction],
    blockhash: Hash,
    l_alt: &[AddressLookupTable],
    l_keypair: &[&Keypair],
    payer_pubkey: Pubkey,
    buf: &mut [u8],
) -> (Signature, usize) {
    // Scan instruction accounts to find which ALTs are referenced.
    let mut l_needed_alt: Vec<usize> = Vec::new();
    for ix in l_ix {
        for meta in &ix.accounts {
            let account_id = account_id_from_pubkey(&meta.pubkey);
            for (i, alt) in l_alt.iter().enumerate() {
                if alt.m_account.contains_key(&account_id) && !l_needed_alt.contains(&i) {
                    l_needed_alt.push(i);
                }
            }
        }
    }

    // Build the ALT accounts directly from stored pubkeys — no cache needed.
    let l_alt_account: Vec<solana_sdk::message::AddressLookupTableAccount> = l_needed_alt
        .iter()
        .map(|&i| {
            let alt = &l_alt[i];
            solana_sdk::message::AddressLookupTableAccount {
                key: solana_sdk::message::Address::from(alt.key_pubkey.to_bytes()),
                addresses: alt
                    .l_pubkey
                    .iter()
                    .map(|pk| solana_sdk::message::Address::from(pk.to_bytes()))
                    .collect(),
            }
        })
        .collect();

    if !l_alt_account.is_empty() {
        let v0_msg = v0::Message::try_compile(
            &solana_sdk::message::Address::from(payer_pubkey.to_bytes()),
            l_ix,
            &l_alt_account,
            blockhash,
        )
        .unwrap();
        let mut vtx =
            VersionedTransaction::try_new(VersionedMessage::V0(v0_msg), l_keypair).unwrap();
        let signature = *vtx.signatures.first().unwrap();
        let size =
            bincode::serde::encode_into_slice(&mut vtx, buf, bincode::config::standard()).unwrap();
        (signature, size)
    } else {
        let mut tx =
            Transaction::new_signed_with_payer(l_ix, Some(&payer_pubkey), l_keypair, blockhash);
        let signature = *tx.signatures.first().unwrap();
        let size =
            bincode::serde::encode_into_slice(&mut tx, buf, bincode::config::standard()).unwrap();
        (signature, size)
    }
}

fn make_ata_instruction(payer: &Pubkey, wallet_owner: &Pubkey, token_mint: &Pubkey) -> Instruction {
    create_associated_token_account_idempotent(payer, wallet_owner, token_mint, &spl_token::ID)
}

/// Decodes a durable-nonce account's raw body into its current
/// readiness. `body` is the real on-chain wire format (classic bincode
/// 1.x fixed-width encoding, `bincode::config::legacy()` -- NOT
/// `standard()`, which uses varint and would misdecode `Versions`'
/// plain-derived enum discriminant). An empty body (account doesn't
/// exist on-chain yet) or anything that fails to decode is treated as
/// `Uninitialized` -- safe, since [`Wallet::ensure_bundler_nonce_created`]
/// only ever creates, never overwrites, so a spurious `Uninitialized`
/// costs at most one harmless, network-rejected duplicate-create attempt.
fn decode_nonce_readiness(body: &[u8]) -> NonceReadiness {
    if body.is_empty() {
        return NonceReadiness::Uninitialized;
    }
    match bincode::serde::decode_from_slice::<NonceVersions, _>(body, bincode::config::legacy()) {
        Ok((versions, _)) => match versions.state() {
            NonceAccountState::Initialized(data) => NonceReadiness::Ready(data.blockhash()),
            NonceAccountState::Uninitialized => NonceReadiness::Uninitialized,
        },
        Err(_) => NonceReadiness::Uninitialized,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_ix() -> Instruction {
        Instruction {
            program_id: Pubkey::new_unique(),
            accounts: vec![],
            data: vec![],
        }
    }

    /// Bare-bones `Wallet`, bypassing `Wallet::new()` -- that constructor
    /// calls `AddressLookupTable::load_default()`, which (via
    /// `account_id_from_pubkey`) reaches a wit-bindgen host import that
    /// only resolves inside the real WASM runtime and aborts the whole
    /// test process natively. None of that machinery is relevant to the
    /// instruction-queue rollback behavior under test here.
    fn bare_wallet() -> Wallet {
        Wallet {
            m_key: HashMap::default(),
            q_ix: VecDeque::default(),
            compute: 0,
            token: TokenDatabase::default(),
            sys_id: 0, // AccountId is a plain u64; irrelevant to this test
            tx_data: Box::new([0u8; 4 * 1024]),
            payer: None,
            m_cache_pubkey: HashMap::default(),
            hs_required: HashSet::new(),
            l_alt: Vec::new(),
            priority_level: PriorityLevel::default(),
            desired_landing_high: false,
            l_ata_sub: Vec::new(),
            m_account_usage: HashMap::default(),
            m_bundler_tip: HashMap::default(),
            hs_known_tip_accounts: HashSet::new(),
            l_tip_sub: Vec::new(),
            o_nonce: None,
            o_nonce_sub: None,
            in_atomic_group: false,
            atomic_group_started: false,
        }
    }

    /// Direct answer to the 2026-08-28 question "is the tip transfer not
    /// being signed?" -- exercises `build_ix_list` +
    /// `Transaction::new_signed_with_payer`, the exact signing call
    /// [`build_and_serialize`]'s no-ALT branch makes (the branch a bare
    /// tip transfer with no ALT-resident accounts always takes -- same
    /// one [`Wallet::assemble`] goes through for every real,
    /// confirmed-landing transaction this session, including
    /// [`Wallet::test_send_astralane_tip_batch`]'s tip transfers). Can't
    /// call `build_and_serialize` itself natively -- its ALT-membership
    /// scan unconditionally calls `account_id_from_pubkey`, a
    /// wit-bindgen host import that aborts the process outside the real
    /// WASM runtime (same reason [`bare_wallet`] exists) -- so this
    /// isolates the signing primitive instead. Deserializes the produced
    /// bytes and calls the real
    /// `solana_sdk::transaction::Transaction::verify()` -- a genuine
    /// ed25519 signature check against the message bytes, not just "a
    /// signature-shaped value is present". If the payer's keypair
    /// weren't actually being used to sign (or the wrong message bytes
    /// were being signed), this fails.
    #[test]
    fn a_bare_transfer_built_like_a_tip_transfer_is_correctly_signed() {
        let payer_kp = Keypair::new();
        let tip_pubkey = Pubkey::new_unique();
        let ix =
            solana_system_interface::instruction::transfer(&payer_kp.pubkey(), &tip_pubkey, 10_000);
        let l_ix = build_ix_list(&[(ix, 5_000, true)], 5_000, 0);
        let blockhash = Hash::new_unique();
        let l_keypair: Vec<&Keypair> = vec![&payer_kp];

        let mut tx = Transaction::new_signed_with_payer(
            &l_ix,
            Some(&payer_kp.pubkey()),
            &l_keypair,
            blockhash,
        );
        let signature = *tx.signatures.first().unwrap();
        let mut buf = [0u8; 4 * 1024];
        let size =
            bincode::serde::encode_into_slice(&mut tx, &mut buf, bincode::config::standard())
                .unwrap();

        let (decoded_tx, _): (Transaction, usize) =
            bincode::serde::decode_from_slice(&buf[..size], bincode::config::standard()).unwrap();
        assert_eq!(
            decoded_tx.signatures.len(),
            1,
            "expected exactly one required signer (the payer)"
        );
        assert_eq!(decoded_tx.signatures[0], signature);
        assert_ne!(
            decoded_tx.signatures[0],
            Signature::default(),
            "signature must not be the all-zero placeholder"
        );
        decoded_tx
            .verify()
            .expect("transaction signature must cryptographically verify against the message");
    }

    #[test]
    fn rollback_to_discards_only_instructions_appended_after_the_checkpoint() {
        let mut wallet = bare_wallet();
        wallet.append_ix(dummy_ix(), 1_000);
        wallet.append_ix(dummy_ix(), 2_000);
        let checkpoint = wallet.queue_checkpoint();
        wallet.append_ix(dummy_ix(), 3_000);
        wallet.append_ix(dummy_ix(), 4_000);

        wallet.rollback_to(checkpoint);

        assert_eq!(wallet.instruction_count(), 2);
        assert_eq!(wallet.cu(), 3_000);
    }

    #[test]
    fn rollback_to_a_multi_hop_route_style_partial_failure_discards_the_earlier_successful_hop() {
        // Mirrors a real bug found in an execute_spot_leg-style multi-hop
        // route (2026-08-27): hop 0 succeeds and appends real instructions, hop 1 then fails --
        // the whole route must roll back to nothing, not just stop hop 2+
        // from being glued on.
        let mut wallet = bare_wallet();
        wallet.begin_atomic_group();
        let checkpoint = wallet.queue_checkpoint();
        wallet.append_ix(dummy_ix(), 5_000); // hop 0's real swap instruction
                                             // hop 1 fails here, before appending anything of its own
        wallet.rollback_to(checkpoint);
        wallet.end_atomic_group();

        assert_eq!(wallet.instruction_count(), 0);
        assert_eq!(wallet.cu(), 0);
    }

    #[test]
    fn rollback_to_a_no_op_checkpoint_is_a_no_op() {
        let mut wallet = bare_wallet();
        wallet.append_ix(dummy_ix(), 1_000);
        let checkpoint = wallet.queue_checkpoint();

        wallet.rollback_to(checkpoint);

        assert_eq!(wallet.instruction_count(), 1);
        assert_eq!(wallet.cu(), 1_000);
    }

    #[test]
    fn atomic_group_fits_is_trivially_true_for_an_empty_group() {
        // Exercises only the checkpoint == current length early-return --
        // the real size-probing path needs a live blockhash host import
        // that aborts outside the real WASM runtime, same testability
        // boundary as Wallet::new() (see bare_wallet's doc comment).
        let wallet = bare_wallet();
        let checkpoint = wallet.queue_checkpoint();
        assert!(wallet.atomic_group_fits(checkpoint));
    }

    #[test]
    fn send_transaction_batch_rejects_empty_input() {
        // Exercises only the host-import-free early-return -- the real
        // send path needs the live `transactionprocessor::batch` host
        // import, same testability boundary as every other real-send
        // method in this file.
        let wallet = bare_wallet();
        assert_eq!(
            wallet.send_transaction_batch(&[], Wallet::ASTRALANE_BUNDLER_CODE),
            Err(transactionprocessor::ErrorCode::InvalidArgument)
        );
    }
}
