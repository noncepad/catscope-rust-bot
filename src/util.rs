use std::{cell::UnsafeCell, rc::Rc, sync::OnceLock, time::Instant};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum LogLevel {
    Debug = 0,
    Info = 1,
    Warn = 2,
    Error = 3,
}

pub fn log_level() -> LogLevel {
    static LEVEL: OnceLock<LogLevel> = OnceLock::new();
    *LEVEL.get_or_init(|| match std::env::var("LOG_LEVEL").as_deref() {
        Ok("DEBUG") => LogLevel::Debug,
        Ok("WARN") => LogLevel::Warn,
        Ok("ERROR") => LogLevel::Error,
        _ => LogLevel::Info,
    })
}

pub fn start_time() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

pub struct PacketHolder {
    pub p: StdioPacket,
}

impl PacketHolder {
    pub fn log(&mut self, data: &[u8]) {
        self.p.append(data);
    }
}

struct SyncCell(UnsafeCell<PacketHolder>);
unsafe impl Sync for SyncCell {}
unsafe impl Send for SyncCell {}

static GLOBAL_PACKET: OnceLock<SyncCell> = OnceLock::new();

pub fn packet_holder() -> &'static mut PacketHolder {
    let cell = GLOBAL_PACKET.get_or_init(|| {
        SyncCell(UnsafeCell::new(PacketHolder {
            p: StdioPacket::stderr(),
        }))
    });
    unsafe { &mut *cell.0.get() }
}

#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        if $crate::util::log_level() <= $crate::util::LogLevel::Debug {
            let msg = format!("[DEBUG] [{:.3?}] {}\n", $crate::util::start_time().elapsed(), format_args!($($arg)*));
            $crate::util::packet_holder().log(msg.as_bytes());
        }
    };
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        if $crate::util::log_level() <= $crate::util::LogLevel::Info {
            let msg = format!("[INFO]  [{:.3?}] {}\n", $crate::util::start_time().elapsed(), format_args!($($arg)*));
            $crate::util::packet_holder().log(msg.as_bytes());
        }
    };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        if $crate::util::log_level() <= $crate::util::LogLevel::Warn {
            let msg = format!("[WARN]  [{:.3?}] {}\n", $crate::util::start_time().elapsed(), format_args!($($arg)*));
            $crate::util::packet_holder().log(msg.as_bytes());
        }
    };
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        if $crate::util::log_level() <= $crate::util::LogLevel::Error {
            let msg = format!("[ERROR] [{:.3?}] {}\n", $crate::util::start_time().elapsed(), format_args!($($arg)*));
            $crate::util::packet_holder().log(msg.as_bytes());
        }
    };
}

use crate::{catscope::witbot::shooter, graph::AccountId, stdio::StdioPacket};
use solana_sdk::pubkey::Pubkey;

/// Resolve `account_id`'s `Pubkey` -- cache-first (see
/// [`pubkey_account_id_cache`]), falling back to the real host lookup
/// and caching the result. Every one of this codebase's existing call
/// sites gets memoization transparently this way, without needing to
/// migrate each one to the cache type directly.
pub fn pubkey_from_account_id(account_id: &AccountId) -> Option<Pubkey> {
    pubkey_account_id_cache().pubkey(account_id)
}

/// Uncached primitive behind [`pubkey_from_account_id`] -- the actual
/// host round-trip, called by [`PubkeyAccountIdCache`] on a cache miss.
/// Not `pub`: every caller should go through the cache (either the free
/// function above, or `PubkeyAccountIdCache` directly for the batch
/// forms), never this one, so a fresh call site can't accidentally
/// bypass memoization.
fn raw_pubkey_from_account_id(account_id: &AccountId) -> Option<Pubkey> {
    let data = match shooter::pubkey_map_by_id(&[*account_id]) {
        Ok(x) => x,
        Err(_) => return None,
    };
    // Real, live-confirmed crash (2026-08-27): the host can return `Ok`
    // with a body that *isn't* exactly 32 bytes (observed: empty `[]`)
    // for an `account_id` it can't resolve, rather than an `Err` -- a
    // different failure mode than the `Err(_)` case above, and this
    // used to `.unwrap()` the length conversion, panicking the whole
    // WASM guest. Treated the same as the `Err` case now: any
    // unexpected length just means "not found," not a crash.
    let y: [u8; 32] = data.try_into().ok()?;
    Some(Pubkey::from(y))
}

/// Batch form of [`pubkey_from_account_id`] -- `shooter::pubkey_map_by_id`
/// itself now accepts a batch of ids (real host-side change, not just a
/// guest-side convenience), so this is one host round-trip for the whole
/// slice instead of one call per id. The WIT `result` is all-or-nothing
/// (no per-item success/failure signal), so any error resolves every
/// entry to `None` -- same "any error -> None" behavior the single-item
/// version already has, just applied to the whole batch at once. Used
/// internally by [`PubkeyAccountIdCache::pubkeys`] for its cache misses;
/// exposed directly too for callers that don't want a cache (e.g. a
/// genuinely one-shot batch never looked up again).
pub fn pubkeys_from_account_ids(account_ids: &[AccountId]) -> Vec<Option<Pubkey>> {
    if account_ids.is_empty() {
        return Vec::new();
    }
    match shooter::pubkey_map_by_id(account_ids) {
        // Real, live-confirmed sibling issue to `raw_pubkey_from_account_id`'s
        // own crash (2026-08-27): this WIT call is documented all-or-
        // nothing (no per-item success signal), so a body whose length
        // doesn't match `account_ids.len() * 32` isn't a partial result
        // safe to `chunks_exact` -- treating it that way would silently
        // misalign output index `i` with input id `i` whenever the host
        // resolved fewer than requested. Falls back to the same
        // all-`None` the `Err` branch already uses.
        Ok(data) if data.len() == account_ids.len() * 32 => data
            .chunks_exact(32)
            .map(|chunk| Some(Pubkey::from(<[u8; 32]>::try_from(chunk).unwrap())))
            .collect(),
        Ok(_) | Err(_) => vec![None; account_ids.len()],
    }
}

/// Resolve `pubkey`'s `AccountId` -- cache-first, same shape as
/// [`pubkey_from_account_id`]. Still panics on a host error (unknown
/// pubkey), same as this codebase's original, uncached behavior --
/// every real call site already assumes the pubkey it's resolving is
/// known-valid (build-time-curated mints, derived PDAs, etc.), so a
/// cache layer underneath doesn't change that contract.
pub fn account_id_from_pubkey(pubkey: &Pubkey) -> AccountId {
    pubkey_account_id_cache().account_id(pubkey)
}

/// Uncached primitive behind [`account_id_from_pubkey`] -- see
/// [`raw_pubkey_from_account_id`]'s doc comment for why this isn't
/// `pub`. Unlike `shooter::pubkey_map_by_id`/[`pubkeys_from_account_ids`],
/// `shooter::pubkey_map_by_pubkey` did **not** gain a batched parameter
/// (it still takes exactly one pubkey's bytes) -- only its return type
/// changed, from a single `u64` to `list<u64>`. Real, live-confirmed via
/// `wit/component.wit`'s diff: `pubkey-map-by-id` genuinely became
/// `func(id: list<u64>)`, while `pubkey-map-by-pubkey` stayed
/// `func(pubkey: list<u8>)`. So there is no host-level batching possible
/// in this direction -- a caller resolving many pubkeys still needs one
/// call each, this just takes the first (expected: only) id the host
/// returns for it.
fn raw_account_id_from_pubkey(pubkey: &Pubkey) -> AccountId {
    match shooter::pubkey_map_by_pubkey(pubkey.as_array()) {
        Ok(ids) => ids.into_iter().next().expect("host returned no account id for a known pubkey"),
        Err(e) => panic!("failed to get account_id {e}"),
    }
}

/// Bidirectional, bounded cache over [`pubkey_from_account_id`]/
/// [`account_id_from_pubkey`] -- both are WIT host imports
/// (`shooter::pubkey_map_by_id`/`pubkey_map_by_pubkey`), so a caller that
/// repeatedly resolves the same handful of pubkeys/account ids (PDA
/// derivations, owner lookups, etc.) can skip the host round-trip
/// entirely once a pair has been seen. Once full, the oldest-inserted
/// pair is evicted to make room for a new one (FIFO, not LRU --
/// simplest policy that still bounds memory, no need to reorder on
/// every lookup).
///
/// Capacity starts at [`Self::STARTUP_CAPACITY`] and drops to
/// [`Self::STEADY_STATE_CAPACITY`] once `graph::all_subscriptions_acked()`
/// reports every subscription request sent so far has been acknowledged
/// by the validator (checked lazily on the next [`Self::insert`], no
/// background timer -- this is a single-threaded WASM guest, cooperative
/// only). Real motivation: `testperpv1::StateHelper::build_time_pubkeys`
/// batch-resolves ~105K pubkeys from build.rs's generated tables at
/// startup, well over [`Self::STEADY_STATE_CAPACITY`] -- without a
/// temporarily larger capacity, FIFO eviction would discard most of
/// that burst before `DexState::new()`'s own constructors (which run
/// immediately after) ever got to reuse it. "Every subscription acked"
/// is used instead of a fixed wall-clock window as the "startup burst
/// is over" signal, since it reflects actual bot state rather than a
/// guessed duration -- once every one of the startup subscription
/// burst's (~32K) requests has landed and been confirmed, the bot has
/// settled into steady-state and ongoing operation doesn't need to hold
/// the entire build-time universe forever, so capacity drops back down
/// (evicting any excess immediately, oldest-first) to bound
/// steady-state memory.
///
/// [`pubkey_from_account_id`]/[`account_id_from_pubkey`] themselves
/// already route through [`pubkey_account_id_cache`]'s shared global
/// instance of this type, so every existing call site in this codebase
/// is cached transparently -- most callers never need to touch this
/// type directly. Reach for it directly only for the batch methods
/// ([`Self::account_ids`]/[`Self::pubkeys`]) or a private, non-global
/// cache scoped to one specific use.
pub struct PubkeyAccountIdCache {
    by_pubkey: std::collections::HashMap<Pubkey, AccountId>,
    by_account_id: std::collections::HashMap<AccountId, Pubkey>,
    /// Insertion order, for FIFO eviction -- oldest pair is always at
    /// the front.
    order: std::collections::VecDeque<Pubkey>,
    capacity: usize,
    /// Still watching for `graph::all_subscriptions_acked()` to flip
    /// `true`, at which point `capacity` drops to
    /// [`PubkeyAccountIdCache::STEADY_STATE_CAPACITY`] -- `false` once
    /// that's already happened (or for a cache that never had a startup
    /// burst capacity to begin with, e.g. [`Self::with_fixed_capacity`]).
    awaiting_ack_downgrade: bool,
}

impl PubkeyAccountIdCache {
    /// Steady-state capacity, once the initial startup pre-warm burst
    /// has had a chance to actually be consumed.
    pub const STEADY_STATE_CAPACITY: usize = 10_000;
    /// Temporary capacity during the startup window -- large enough
    /// that `testperpv1`'s ~105K-pubkey build-time batch survives
    /// without FIFO eviction until it's actually reused.
    pub const STARTUP_CAPACITY: usize = 150_000;

    pub fn new() -> Self {
        Self::with_capacity(Self::STARTUP_CAPACITY, true)
    }

    /// Build a cache with a fixed capacity and no startup-burst
    /// downgrade -- for a private, non-global cache scoped to one
    /// specific use that doesn't need the startup-burst behavior
    /// [`Self::new`]'s global instance is tuned for.
    pub fn with_fixed_capacity(capacity: usize) -> Self {
        Self::with_capacity(capacity, false)
    }

    fn with_capacity(capacity: usize, awaiting_ack_downgrade: bool) -> Self {
        Self {
            by_pubkey: std::collections::HashMap::with_capacity(capacity),
            by_account_id: std::collections::HashMap::with_capacity(capacity),
            order: std::collections::VecDeque::with_capacity(capacity),
            capacity,
            awaiting_ack_downgrade,
        }
    }

    /// How many pairs are currently cached.
    pub fn len(&self) -> usize {
        self.by_pubkey.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_pubkey.is_empty()
    }

    /// Current capacity -- see the struct's own doc comment for why
    /// this can change over the cache's lifetime.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Drops `capacity` to [`Self::STEADY_STATE_CAPACITY`] and evicts
    /// down to it immediately (oldest-first), if not already there.
    /// Split out from [`Self::maybe_downgrade_capacity`] so a test can
    /// exercise the eviction mechanics directly without depending on
    /// the real, process-global `graph::all_subscriptions_acked()`.
    fn downgrade_now(&mut self) {
        self.awaiting_ack_downgrade = false;
        self.capacity = Self::STEADY_STATE_CAPACITY;
        while self.order.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                if let Some(id) = self.by_pubkey.remove(&oldest) {
                    self.by_account_id.remove(&id);
                }
            }
        }
    }

    /// If still waiting for the startup subscription burst to finish
    /// (`graph::all_subscriptions_acked()`) and it now has, downgrades
    /// capacity -- see the struct's own doc comment.
    fn maybe_downgrade_capacity(&mut self) {
        if self.awaiting_ack_downgrade && crate::graph::all_subscriptions_acked() {
            self.downgrade_now();
        }
    }

    /// Records `pubkey <-> account_id`, evicting the oldest pair first
    /// if already at capacity. No-op if `pubkey` is already cached
    /// (keeps its original insertion-order position).
    fn insert(&mut self, pubkey: Pubkey, account_id: AccountId) {
        self.maybe_downgrade_capacity();
        if self.by_pubkey.contains_key(&pubkey) {
            return;
        }
        if self.order.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                if let Some(id) = self.by_pubkey.remove(&oldest) {
                    self.by_account_id.remove(&id);
                }
            }
        }
        self.by_pubkey.insert(pubkey, account_id);
        self.by_account_id.insert(account_id, pubkey);
        self.order.push_back(pubkey);
    }

    /// Resolve `pubkey`'s `AccountId` -- served from the cache if
    /// present, otherwise falls back to the real host lookup (via
    /// [`raw_account_id_from_pubkey`]) and caches the result.
    pub fn account_id(&mut self, pubkey: &Pubkey) -> AccountId {
        if let Some(&id) = self.by_pubkey.get(pubkey) {
            return id;
        }
        let id = raw_account_id_from_pubkey(pubkey);
        self.insert(*pubkey, id);
        id
    }

    /// Batch form of [`Self::account_id`]. Still one host call per
    /// cache-miss (`shooter::pubkey_map_by_pubkey` never gained a
    /// batched parameter -- see [`raw_account_id_from_pubkey`]'s doc
    /// comment), but repeats already resolve for free from the cache.
    pub fn account_ids(&mut self, pubkeys: &[Pubkey]) -> Vec<AccountId> {
        pubkeys.iter().map(|pk| self.account_id(pk)).collect()
    }

    /// Resolve `account_id`'s `Pubkey` -- served from the cache if
    /// present, otherwise falls back to the real host lookup (via
    /// [`raw_pubkey_from_account_id`]) and caches the result if found.
    /// `None` if the host has no mapping for `account_id`.
    pub fn pubkey(&mut self, account_id: &AccountId) -> Option<Pubkey> {
        if let Some(&pk) = self.by_account_id.get(account_id) {
            return Some(pk);
        }
        let pk = raw_pubkey_from_account_id(account_id)?;
        self.insert(pk, *account_id);
        Some(pk)
    }

    /// Batch form of [`Self::pubkey`] -- unlike [`Self::account_ids`],
    /// this makes at most **one** real host call
    /// ([`pubkeys_from_account_ids`]) for every cache-missed id at once,
    /// since `shooter::pubkey_map_by_id` genuinely accepts a batch (see
    /// that function's doc comment). Cache hits never touch the host at
    /// all. Results are returned in the same order as `account_ids`.
    pub fn pubkeys(&mut self, account_ids: &[AccountId]) -> Vec<Option<Pubkey>> {
        let mut out = vec![None; account_ids.len()];
        let mut miss_positions = Vec::new();
        let mut miss_ids = Vec::new();
        for (i, id) in account_ids.iter().enumerate() {
            if let Some(&pk) = self.by_account_id.get(id) {
                out[i] = Some(pk);
            } else {
                miss_positions.push(i);
                miss_ids.push(*id);
            }
        }
        if miss_ids.is_empty() {
            return out;
        }
        let resolved = pubkeys_from_account_ids(&miss_ids);
        for ((pos, id), pk) in miss_positions.into_iter().zip(miss_ids).zip(resolved) {
            if let Some(pk) = pk {
                self.insert(pk, id);
            }
            out[pos] = pk;
        }
        out
    }
}

impl Default for PubkeyAccountIdCache {
    fn default() -> Self {
        Self::new()
    }
}

struct SyncPubkeyAccountIdCache(UnsafeCell<PubkeyAccountIdCache>);
unsafe impl Sync for SyncPubkeyAccountIdCache {}
unsafe impl Send for SyncPubkeyAccountIdCache {}

static GLOBAL_PUBKEY_ACCOUNT_ID_CACHE: OnceLock<SyncPubkeyAccountIdCache> = OnceLock::new();

/// Process-wide [`PubkeyAccountIdCache`] instance -- same
/// lazily-initialized global-singleton shape as [`packet_holder`], for
/// the same reason: every `account_id_from_pubkey`/`pubkey_from_account_id`
/// call site across this codebase should share one cache rather than
/// each maintaining its own (which would just multiply host round-trips
/// for the same pubkeys/ids looked up from different modules). Safe in
/// this single-threaded WASM guest, same as every other global here.
pub fn pubkey_account_id_cache() -> &'static mut PubkeyAccountIdCache {
    let cell =
        GLOBAL_PUBKEY_ACCOUNT_ID_CACHE.get_or_init(|| SyncPubkeyAccountIdCache(UnsafeCell::new(PubkeyAccountIdCache::new())));
    unsafe { &mut *cell.0.get() }
}

/// Look up `symbol` (case-sensitive, e.g. `"SOL"`, not `"SOL-PERP"`)
/// against the build-time-curated `symbol_mint_config::SYMBOL_MINT_MAP`,
/// resolving to an `AccountId` via `account_id_from_pubkey`. Returns
/// `None` for a symbol with no curated entry -- shared by
/// `dex::velocity::state`/`dex::phoenix` to attach a `base_mint` join
/// key onto perp market data, which genuinely has no mint reference
/// on-chain (see `VelocityPerpMarketView::base_mint`'s doc comment).
pub fn resolve_symbol_mint(symbol: &str) -> Option<AccountId> {
    crate::symbol_mint_config::SYMBOL_MINT_MAP
        .iter()
        .find(|entry| {
            let entry_symbol = std::str::from_utf8(&entry.symbol)
                .unwrap_or("")
                .trim_end_matches('\0');
            entry_symbol == symbol
        })
        .map(|entry| account_id_from_pubkey(&Pubkey::new_from_array(entry.mint)))
}

/// Same lookup as `resolve_symbol_mint`, for `symbol`'s mint decimals --
/// build.rs-baked from the real `mint_info` table (see
/// `symbol_mint_config::SYMBOL_MINT_MAP`'s generation in `build.rs`),
/// needed to convert a USD amount to/from raw token units (e.g.
/// `perpfundingv1::state::StateHelper::rebalance_portfolio`'s trade
/// sizing).
pub fn resolve_symbol_decimals(symbol: &str) -> Option<u8> {
    crate::symbol_mint_config::SYMBOL_MINT_MAP
        .iter()
        .find(|entry| {
            let entry_symbol = std::str::from_utf8(&entry.symbol)
                .unwrap_or("")
                .trim_end_matches('\0');
            entry_symbol == symbol
        })
        .map(|entry| entry.decimals)
}

#[inline]
pub fn as_bytes_mut<T: Sized>(val: &mut T) -> &mut [u8] {
    unsafe { std::slice::from_raw_parts_mut((val as *mut T) as *mut u8, std::mem::size_of::<T>()) }
}

#[inline]
pub fn as_bytes<T: Sized>(val: &T) -> &[u8] {
    unsafe { std::slice::from_raw_parts((val as *const T) as *const u8, std::mem::size_of::<T>()) }
}

#[inline]
pub fn rc_unlock_mut<'a, 'b: 'a, T>(object: &'b Rc<UnsafeCell<T>>) -> &'a mut T {
    unsafe { &mut *object.get() }
}

#[inline]
pub fn rc_unlock<'a, 'b: 'a, T>(object: &'b Rc<UnsafeCell<T>>) -> &'a T {
    unsafe { &*object.get() }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pure data check, no host import -- `resolve_symbol_mint`'s
    /// `Some` path can't be exercised in a native unit test
    /// (`account_id_from_pubkey` is a WIT host import that aborts
    /// outside the real WASM guest runtime, same established boundary
    /// as every other `Wallet`/pubkey-resolution code in this codebase).
    /// This at least confirms the curated table actually contains SOL.
    #[test]
    fn symbol_mint_map_contains_sol() {
        let has_sol = crate::symbol_mint_config::SYMBOL_MINT_MAP
            .iter()
            .any(|entry| {
                std::str::from_utf8(&entry.symbol)
                    .unwrap_or("")
                    .trim_end_matches('\0')
                    == "SOL"
            });
        assert!(has_sol, "SYMBOL_MINT_MAP should contain a SOL entry");
    }

    /// Unlike the `Some` path, this never reaches `account_id_from_pubkey`
    /// -- `.find()` fails before `.map()` ever runs -- so it's safe to
    /// run natively.
    #[test]
    fn resolve_symbol_mint_none_for_unknown_symbol() {
        assert_eq!(resolve_symbol_mint("NOT_A_REAL_SYMBOL"), None);
    }

    /// Unlike `resolve_symbol_mint`, this never reaches
    /// `account_id_from_pubkey` at all -- `decimals` is a plain `u8`
    /// field, no pubkey resolution -- so the `Some` path is safe to
    /// exercise natively too.
    #[test]
    fn resolve_symbol_decimals_matches_real_mint_info() {
        // Verified against the real prefetch.db's mint_info table this
        // session: SOL=9, BTC=8, ETH=8, XRP=6, BNB=8, SUI=8.
        assert_eq!(resolve_symbol_decimals("SOL"), Some(9));
        assert_eq!(resolve_symbol_decimals("XRP"), Some(6));
    }

    #[test]
    fn resolve_symbol_decimals_none_for_unknown_symbol() {
        assert_eq!(resolve_symbol_decimals("NOT_A_REAL_SYMBOL"), None);
    }

    /// Exercises the cache-hit path only, via `insert` directly --
    /// `account_id`/`pubkey`'s cache-miss fallback hits the same
    /// `account_id_from_pubkey`/`pubkey_from_account_id` host-import
    /// boundary as everything else in this file, so it can't be reached
    /// in a native unit test.
    #[test]
    fn pubkey_account_id_cache_hits_both_directions() {
        let mut cache = PubkeyAccountIdCache::new();
        let pk = Pubkey::new_unique();
        cache.insert(pk, 42);
        assert_eq!(cache.account_id(&pk), 42);
        assert_eq!(cache.pubkey(&42), Some(pk));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn pubkey_account_id_cache_batch_lookups_match_single_lookups() {
        let mut cache = PubkeyAccountIdCache::new();
        let pks: Vec<Pubkey> = (0..5).map(|_| Pubkey::new_unique()).collect();
        for (i, pk) in pks.iter().enumerate() {
            cache.insert(*pk, i as AccountId);
        }
        assert_eq!(cache.account_ids(&pks), vec![0, 1, 2, 3, 4]);
        let ids: Vec<AccountId> = (0..5).collect();
        assert_eq!(
            cache.pubkeys(&ids),
            pks.iter().map(|pk| Some(*pk)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn pubkey_account_id_cache_repeated_insert_is_a_no_op() {
        let mut cache = PubkeyAccountIdCache::new();
        let pk = Pubkey::new_unique();
        cache.insert(pk, 1);
        cache.insert(pk, 2);
        assert_eq!(cache.account_id(&pk), 1, "first insert should win, not be overwritten");
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn pubkey_account_id_cache_evicts_oldest_once_full() {
        let capacity = 100;
        let mut cache = PubkeyAccountIdCache::with_fixed_capacity(capacity);
        let pks: Vec<Pubkey> = (0..capacity + 1).map(|_| Pubkey::new_unique()).collect();
        for (i, pk) in pks.iter().enumerate() {
            cache.insert(*pk, i as AccountId);
        }
        assert_eq!(cache.len(), capacity);
        // The very first pair inserted should have been evicted to make
        // room for the last one -- checked via internal state directly
        // (not `pubkey(&0)`/`account_id(&pks[0])`, which would fall
        // through to the real host-import lookup on a genuine miss and
        // abort in a native test, same boundary every other test in
        // this file avoids).
        assert!(!cache.by_account_id.contains_key(&0));
        assert!(!cache.by_pubkey.contains_key(&pks[0]));
        assert_eq!(cache.account_id(&pks[capacity]), capacity as AccountId);
    }

    /// `PubkeyAccountIdCache::new` starts at `STARTUP_CAPACITY` so a
    /// large startup pre-warm (e.g. `testperpv1`'s ~105K build-time
    /// pubkeys) survives without eviction; once
    /// `graph::all_subscriptions_acked()` reports the startup
    /// subscription burst fully acked, capacity should drop to
    /// `STEADY_STATE_CAPACITY` and evict any excess immediately.
    /// Exercises `downgrade_now` directly (private, visible from this
    /// child module) rather than going through `new()`'s real
    /// `awaiting_ack_downgrade` path -- `graph::all_subscriptions_acked()`
    /// reads process-global atomics shared with every other test in
    /// this binary (Rust tests run in the same process, often in
    /// parallel), so depending on its real value here would make this
    /// test's outcome depend on unrelated tests' subscription activity.
    #[test]
    fn pubkey_account_id_cache_downgrades_capacity_once_triggered() {
        let mut cache = PubkeyAccountIdCache::with_capacity(PubkeyAccountIdCache::STEADY_STATE_CAPACITY * 2, true);
        for i in 0..(PubkeyAccountIdCache::STEADY_STATE_CAPACITY + 5) as AccountId {
            cache.insert(Pubkey::new_unique(), i);
        }
        assert_eq!(
            cache.len(),
            PubkeyAccountIdCache::STEADY_STATE_CAPACITY + 5,
            "still within the doubled startup capacity, nothing evicted yet"
        );
        assert_eq!(cache.capacity(), PubkeyAccountIdCache::STEADY_STATE_CAPACITY * 2);

        cache.downgrade_now();

        assert_eq!(cache.capacity(), PubkeyAccountIdCache::STEADY_STATE_CAPACITY);
        assert_eq!(
            cache.len(),
            PubkeyAccountIdCache::STEADY_STATE_CAPACITY,
            "should have evicted down to the new, smaller capacity immediately"
        );
    }
}
