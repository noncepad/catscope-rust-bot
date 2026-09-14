use crate::{
    catscope::witbot::shooter::{
        self, Accountv1, Client, Commit as ShooterCommit, Header, Tokenaccountv1,
    },
    err::CatscopeGuestError,
    event::{Event, EventCallback},
    event_loop::EventPoller,
    log_warn,
    txview::TransactionList,
    util::as_bytes_mut,
};
use crate::{log_debug, log_info};
use solana_sdk::clock::Slot;
use std::{
    cell::{RefCell, UnsafeCell},
    collections::HashSet,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

pub type AccountId = u64;

/// Total subscription ids ever sent (`Graph::bulk_subscribe`/`subscribe`
/// -- every id in every successful call's result), and total acks ever
/// received for them (`eventitem.ack`, wired up in
/// `Graph::on_event` below -- previously read but never used, per this
/// session's earlier research into `wit/component.wit`'s ack mechanism).
/// Process-wide, not per-`Graph` instance, since callers besides
/// `testperpv1` create their own `Graph`/subscribe independently and the
/// consumer of this signal ([`all_subscriptions_acked`]) needs a single
/// global answer either way.
static SUBSCRIPTIONS_SENT: AtomicU64 = AtomicU64::new(0);
static SUBSCRIPTIONS_ACKED: AtomicU64 = AtomicU64::new(0);

/// Whether every subscription id sent so far has been acknowledged by
/// the validator. `false` until at least one has been sent *and* every
/// one sent has come back acked -- real motivation:
/// `util::PubkeyAccountIdCache`'s startup-capacity downgrade (see its
/// own doc comment) uses this as the "the startup subscription burst is
/// done" signal.
pub fn all_subscriptions_acked() -> bool {
    let sent = SUBSCRIPTIONS_SENT.load(Ordering::Relaxed);
    sent > 0 && SUBSCRIPTIONS_ACKED.load(Ordering::Relaxed) >= sent
}

/// Raw `(acked, sent)` subscription counts backing
/// [`all_subscriptions_acked`] -- exposed for diagnostic logging of real
/// sync progress (e.g. "38214/41982 acked, still catching up") instead of
/// just the coarse all-or-nothing boolean. Acked here means the
/// *subscribe request itself* was acknowledged by the validator/gateway,
/// not that the account's first real data update has arrived yet -- see
/// this module's `SUBSCRIPTIONS_SENT`/`SUBSCRIPTIONS_ACKED` doc comment.
pub fn subscription_ack_counts() -> (u64, u64) {
    (
        SUBSCRIPTIONS_ACKED.load(Ordering::Relaxed),
        SUBSCRIPTIONS_SENT.load(Ordering::Relaxed),
    )
}
pub type Weight = u32;
pub type Depth = u8;
pub type TokenAmount = u64;
pub type Lamports = u64;

/// Accounts requested per bigcommit.read() call while draining a large commit.
const BIGCOMMIT_BATCH_SIZE: u32 = 2_000;

#[derive(Clone)]
pub struct Graph {
    poller: EventPoller,
    inner: Rc<RefCell<InnerGraph>>,
    read_count: usize,
}

#[derive(Debug)]
struct InnerGraph {
    graph_event_id: u32,
    client: Client,
    hs_sub: HashSet<u32>,
    poller: EventPoller,
}
impl Drop for InnerGraph {
    fn drop(&mut self) {
        assert!(self.hs_sub.is_empty());
        self.poller.unregister(self.graph_event_id);
    }
}

impl Graph {
    /// Create a new connection to the Catscope account store.
    /// The connection is canceled once the grpah variable and all subscription
    /// objects are dropped.
    pub fn new(poller: EventPoller) -> Result<Rc<UnsafeCell<Self>>, CatscopeGuestError> {
        let client = match shooter::connect() {
            Ok(x) => x,
            Err(e) => return Err(CatscopeGuestError::Shooter(e)),
        };
        let event_id = client.poll();
        let inner = Rc::new(RefCell::new(InnerGraph {
            graph_event_id: event_id,
            hs_sub: HashSet::with_capacity(10),
            client,
            poller: poller.clone(),
        }));
        let g = Self {
            poller: poller.clone(),
            inner,
            read_count: 0,
        };
        let g1 = Rc::new(UnsafeCell::new(g));
        poller.register(event_id, g1.clone());
        Ok(g1)
    }
    /// Add a graph subset to streaming updates. Deliberately **not**
    /// `pub` -- `bulk_subscribe`/`subscribe` (the WIT `bulksubscribe`
    /// call underneath) are confirmed blocking calls on the validator
    /// side, and a caller building its own unbounded/unpaced batch is
    /// exactly the bug that produced this session's real `stdio timeout`
    /// hangs (traced to `flush_pool`'s per-sub-dex accumulators each
    /// firing their own uncapped `bulk_subscribe` call). Every caller
    /// must go through [`SubscriptionQueue`] instead -- either its paced
    /// [`SubscriptionQueue::flush`] (bounded, spread across slots) or,
    /// for genuinely small/fixed/immediate batches that need results
    /// correlated back synchronously, [`SubscriptionQueue::subscribe_now`].
    /// Module-private on purpose: only code in this file can reach this
    /// method, so that invariant is enforced by the compiler, not just
    /// convention.
    fn bulk_subscribe(
        &self,
        l_req: Vec<SubscriptionRequest>,
    ) -> Result<Vec<Subscription>, CatscopeGuestError> {
        let mx = self.inner.borrow();
        let mut l_full = Vec::with_capacity(l_req.len());
        for req in l_req {
            l_full.push((req.root, req.filter_weight, req.depth as u32));
        }
        let l_sub_id = mx.client.bulksubscribe(l_full.as_slice())?;
        SUBSCRIPTIONS_SENT.fetch_add(l_sub_id.len() as u64, Ordering::Relaxed);
        let mut l_sub = Vec::with_capacity(l_sub_id.len());
        for id in l_sub_id {
            l_sub.push(Subscription {
                id,
                inner: self.inner.clone(),
            });
        }
        Ok(l_sub)
    }
    /// Add a graph subset to streaming updates.
    pub fn subscribe(&self, req: SubscriptionRequest) -> Result<Subscription, CatscopeGuestError> {
        let mx = self.inner.borrow();
        let id = mx
            .client
            .subscribe(req.root, req.filter_weight, req.depth as u32)?;
        SUBSCRIPTIONS_SENT.fetch_add(1, Ordering::Relaxed);
        Ok(Subscription {
            id,
            inner: self.inner.clone(),
        })
    }
}
#[derive(Debug)]
pub struct SubscriptionRequest {
    pub root: AccountId,
    pub filter_weight: Weight,
    pub depth: Depth,
}
impl EventCallback for Graph {
    fn on_event(&mut self) -> Result<bool, CatscopeGuestError> {
        let mx = self.inner.borrow();
        self.read_count += 1;
        let mut item = mx.client.read();
        let l_ack = item.ack;
        let o_slot = item.commitslot;
        let o_commit = item.commit;
        let l_sws = item.slotstatus;

        if item.accountdata.is_some() || item.tokendata.is_some() {
            let tokendata = item.tokendata.take().unwrap_or_default();
            let tokenborder = item.tokenborder.take().unwrap_or_default();
            let accountdata = item.accountdata.take().unwrap_or_default();
            let accountborder = item.accountborder.take().unwrap_or_default();
            log_debug!("Graph::on_event - 2");
            self.poller
                .event(Event::LowLatency(LowLatencyAccountUpdate {
                    token_i: 0,
                    tokendata,
                    last_token_i: 0,
                    tokenborder,
                    account_i: 0,
                    last_account_i: 0,
                    accountdata,
                    accountborder,
                }));
        }

        if let Some(txdata) = item.txdata.take() {
            let txborder = item.txborder.take().unwrap();
            log_debug!("Graph::on_event - 4");
            self.poller
                .event(Event::Transaction(TransactionList::new(txdata, txborder)));
        }

        for sws in &l_sws {
            self.poller
                .event(Event::SlotStatus(sws.slot, sws.status.try_into().unwrap()));
        }

        if let Some(bigcommit) = o_commit {
            // A commit too large to deliver in one eventitem arrives as a
            // bigcommit resource instead of an inline commit record. Drain it
            // in batches and reassemble one combined commit — border offsets
            // in each batch are relative to that batch's own data, so they
            // need to be rebased onto the concatenated buffer as we go.
            let slot = bigcommit.slot();
            let mut data = Vec::new();
            let mut border = Vec::new();
            loop {
                let batch = bigcommit.read(BIGCOMMIT_BATCH_SIZE);
                let base = data.len() as u32;
                for b in &batch.border {
                    border.push(*b + base);
                }
                data.extend_from_slice(&batch.data);
                if bigcommit.done() {
                    break;
                }
            }
            log_warn!(
                "Graph::on_event - 5 - border {}; data {}; read_count {}",
                border.len(),
                data.len(),
                self.read_count,
            );
            self.read_count = 0;
            let inner_commit = ShooterCommit { slot, data, border };
            self.poller.event(Event::Commit(Commit::new(inner_commit)));
        } else if let Some(slot) = o_slot {
            if slot.is_multiple_of(10) {
                log_warn!("Graph::on_event - 6 - read_count {}", self.read_count,);
            }
            self.read_count = 0;
            let inner_commit = ShooterCommit {
                slot,
                data: vec![],
                border: vec![],
            };
            self.poller.event(Event::Commit(Commit::new(inner_commit)));
        }

        if !l_ack.is_empty() {
            SUBSCRIPTIONS_ACKED.fetch_add(l_ack.len() as u64, Ordering::Relaxed);
            log_debug!("Graph::on_event - 7 - {l_ack:?}");
        }

        Ok(true)
    }
}

fn parse_tokens(data: &[u8], borders: &[u32]) -> Vec<Tokenaccountv1> {
    let token_len = std::mem::size_of::<Tokenaccountv1>();
    let mut out = Vec::with_capacity(borders.len());
    let mut tok = Tokenaccountv1 {
        id: 0,
        owner: 0,
        mint: 0,
        amount: 0,
        slot: 0,
        version: 0,
    };
    let mut start = 0usize;
    for &end in borders {
        let end2 = end as usize;
        assert_eq!(end2 - start, token_len);
        as_bytes_mut(&mut tok).copy_from_slice(&data[start..end2]);
        out.push(Tokenaccountv1 {
            id: tok.id,
            owner: tok.owner,
            mint: tok.mint,
            amount: tok.amount,
            slot: tok.slot,
            version: tok.version,
        });
        start = end2;
    }
    out
}

fn parse_accounts(data: &[u8], borders: &[u32]) -> Vec<Accountv1> {
    let header_len = std::mem::size_of::<Header>();
    let mut out = Vec::with_capacity(borders.len());
    let mut hdr = Header {
        slot: 0,
        version: 0,
        lamports: 0,
        accountid: 0,
        owner: 0,
        datasize: 0,
    };
    let mut start = 0usize;
    for &end in borders {
        let end2 = end as usize;
        as_bytes_mut(&mut hdr).copy_from_slice(&data[start..start + header_len]);
        let body = data[start + header_len..end2].to_vec();
        out.push(Accountv1 {
            header: Header {
                slot: hdr.slot,
                version: hdr.version,
                lamports: hdr.lamports,
                accountid: hdr.accountid,
                owner: hdr.owner,
                datasize: hdr.datasize,
            },
            body,
        });
        start = end2;
    }
    out
}

pub struct Commit {
    commit: ShooterCommit,
}

impl Commit {
    fn new(sc: ShooterCommit) -> Self {
        if sc.slot % 100 == 0 {
            log_info!("shooter commit {} - 1", sc.slot);
        }
        Self { commit: sc }
    }

    /// Process a commit.
    pub fn process<CH: CommitHook>(&self, hook: &mut CH) {
        let slot = self.commit.slot;
        if slot % 100 == 0 {
            log_warn!("commit - 1 - slot {slot}");
        }
        hook.start(slot);
        let data = &self.commit.data;
        log_debug!(
            "commit - 2 - ____________slot {}; data {}",
            slot,
            data.len()
        );
        let zerodata: [u8; 0] = [];
        let l_border = &self.commit.border;
        log_debug!(
            "commit - 2 - slot {}; data {}; border {:?}",
            slot,
            data.len(),
            l_border
        );
        let header_len = std::mem::size_of::<Header>();
        let token_len = std::mem::size_of::<Tokenaccountv1>();

        let mut header = Header {
            slot,
            lamports: 0,
            version: 0,
            accountid: 0,
            owner: 0,
            datasize: 0,
        };
        let mut token_account = Tokenaccountv1 {
            id: 0,
            owner: 0,
            mint: 0,
            amount: 0,
            slot,
            version: 0,
        };
        let mut start = 0;
        let mut finish;
        let border_count = l_border.len();
        let mut token_count = 0;
        for (i, f1) in l_border.iter().enumerate() {
            if i.is_multiple_of(1_000) {
                log_warn!(
                    "commit:border {}/{}; token {}; other {}",
                    i,
                    border_count,
                    token_count,
                    border_count - token_count
                );
            }
            // Two checkpoints (1/2, 3/4) -- live evidence narrowed a real
            // hang to somewhere in the second half of this loop (border
            // and the 1/2 checkpoint both fired, `commit:done` never
            // did), so the 3/4 mark bisects that now-known region
            // further; the 1/2 mark stays for symmetry/regression watch.
            let is_checkpoint =
                border_count != 0 && (i == border_count / 2 || i == 3 * border_count / 4);
            finish = *f1 as usize;
            if finish - start == token_len {
                let subbuf = &data[start..finish];
                let dst_buf = as_bytes_mut(&mut token_account);
                dst_buf.copy_from_slice(subbuf);
                token_count += 1;
                hook.on_token(&token_account);
                // One extra checkpoint at the loop's midpoint -- same
                // proportional-volume reasoning as `commit:border`/
                // `commit:done` (one more line per commit, not a new
                // per-record log). Bisects a hang localized to *inside*
                // this loop (border fires, done doesn't) down to which
                // half, and which specific record was just dispatched
                // right before it -- real, live-observed need: after
                // routing every `bulk_subscribe` call through
                // `SubscriptionQueue`, a hang was still traced to
                // somewhere in this loop for an otherwise ordinary-sized
                // commit.
                if is_checkpoint {
                    log_warn!(
                        "commit:mid - slot {slot}; border {i}/{border_count}; last dispatched token owner={} mint={}",
                        token_account.owner,
                        token_account.mint,
                    );
                }
            } else if header_len <= finish - start {
                let header_subbuf = &data[start..(start + header_len)];
                let dst_buf = as_bytes_mut(&mut header);
                dst_buf.copy_from_slice(header_subbuf);
                log_debug!("commmit: {header:?}__");
                let body_size = header.datasize as usize;
                if header_len + body_size != finish - start {
                    panic!(
                        "bad account length; start {}; header_len {}; body_size {}; pubkey {}; slot {}; lamports {}",
                        start, header_len, body_size,header.accountid,header.slot,header.lamports
                    )
                }
                let d = if 0 < body_size {
                    &data[(start + header_len)..finish]
                } else {
                    &zerodata
                };

                hook.on_account(&header, d);
                // See the matching checkpoint in the token branch above.
                if is_checkpoint {
                    log_warn!(
                        "commit:mid - slot {slot}; border {i}/{border_count}; last dispatched account accountid={} owner={}",
                        header.accountid,
                        header.owner,
                    );
                }
            } else {
                panic!("bad account length")
            }

            start = finish;
        }
        // Bookends the `commit:border 0/{border_count}; ...` line logged
        // at the top of this loop -- that line already fires once per
        // commit in practice (border_count is always well under the
        // 1_000 progress-log interval for this codebase's real commits),
        // so this is a matched pair, not a new log-volume source. Lets a
        // real hang be localized: if a commit's `commit:border` line
        // appears but this one never does, the freeze is inside the
        // on_account/on_token loop above; if both appear but nothing
        // from `hook.finish()` ever follows, the freeze is inside
        // `finish()` instead (e.g. a `bulk_subscribe` call that never
        // returns).
        log_warn!(
            "commit:done - slot {slot}; border {border_count}/{border_count}; token {token_count}; other {}",
            border_count - token_count
        );
        hook.finish();
    }
}

/// Keep a subscription alive.
/// Once this object drops, the subscription is canceled.
#[derive(Debug)]
pub struct Subscription {
    id: u32,
    inner: Rc<RefCell<InnerGraph>>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let mut mx = self.inner.borrow_mut();
        if mx.hs_sub.remove(&self.id) {
            mx.client.cancel(self.id);
        }
    }
}

/// Paces subscription requests across multiple slots instead of firing
/// them all in one `bulk_subscribe` call. Real, live-observed motivation:
/// `subscribe`/`bulk_subscribe` are blocking calls on the validator side
/// -- a single `bulk_subscribe` for, say, ~32,000 accounts (this
/// codebase's own real Raydium/Orca/lending-reserve startup burst)
/// blocks the guest for however long the host takes to service the
/// *entire* batch in one round-trip, not just this account's own real
/// work. Queue requests here as they're discovered (`push`/`extend`),
/// then call [`Self::flush`] once per slot -- typically from
/// `CommitHook::finish` -- to drain a bounded number of them into one
/// `bulk_subscribe` call instead.
///
/// Resulting [`Subscription`]s are kept alive internally; nothing else
/// needs to hold onto them (same "just don't let it drop" role every
/// existing `_subscriptions: Vec<Subscription>` field in this codebase
/// already plays, e.g. `RaydiumAmm`/`OrcaState`) -- the request's
/// *meaning* (which pool/reserve/account a given `AccountId` maps to)
/// is expected to already be tracked separately by the caller (every
/// existing `Xxx::new` in this codebase already builds its own
/// `m_pool`/`m_bank`-style map keyed by `AccountId` before subscribing,
/// independent of the `Subscription` handle itself), so this queue
/// doesn't need to correlate results back to individual requests.
#[derive(Debug, Default)]
pub struct SubscriptionQueue {
    pending: std::collections::VecDeque<SubscriptionRequest>,
    subscriptions: Vec<Subscription>,
}

impl SubscriptionQueue {
    pub fn push(&mut self, req: SubscriptionRequest) {
        self.pending.push_back(req);
    }

    pub fn extend(&mut self, reqs: impl IntoIterator<Item = SubscriptionRequest>) {
        self.pending.extend(reqs);
    }

    /// How many requests are still queued, not yet sent.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// How many subscriptions this queue has sent and is keeping alive.
    pub fn active_count(&self) -> usize {
        self.subscriptions.len()
    }

    /// Drain up to `max_per_flush` queued requests into one bounded
    /// `bulk_subscribe` call, keeping the resulting subscriptions alive
    /// internally. No-op (`Ok(0)`) if the queue is empty. Returns how
    /// many requests this call actually sent.
    pub fn flush(&mut self, g: &Graph, max_per_flush: usize) -> Result<usize, CatscopeGuestError> {
        if self.pending.is_empty() || max_per_flush == 0 {
            return Ok(0);
        }
        let n = self.pending.len().min(max_per_flush);
        let batch: Vec<SubscriptionRequest> = self.pending.drain(..n).collect();
        let subs = g.bulk_subscribe(batch)?;
        self.subscriptions.extend(subs);
        Ok(n)
    }

    /// Subscribe to every one of `reqs` immediately, in one bounded
    /// `bulk_subscribe` call, returning the raw [`Subscription`] handles
    /// instead of retaining them internally -- for callers that need
    /// per-request correlation `flush`'s "just keep them alive" contract
    /// can't give them (e.g. wallet-authority tracking, which hands each
    /// resulting `Subscription` back to whichever protocol's request
    /// produced it), or a small fixed one-shot batch that doesn't need
    /// pacing across slots (e.g. a `PhoenixMarket`'s 3-item second-hop
    /// discovery). Still the *only* other path to the underlying host
    /// call besides `flush` -- callers are responsible for keeping
    /// `reqs.len()` small, since this is not paced.
    pub fn subscribe_now(
        g: &Graph,
        reqs: Vec<SubscriptionRequest>,
    ) -> Result<Vec<Subscription>, CatscopeGuestError> {
        g.bulk_subscribe(reqs)
    }
}

/// Process finalized account state.
pub trait CommitHook {
    fn start(&mut self, slot: Slot);
    fn on_account(&mut self, header: &Header, body: &[u8]);
    fn on_token(&mut self, token_account: &Tokenaccountv1);
    fn finish(&mut self);
}
pub struct AccountRef<'a> {
    pub header: &'a Header,
    pub body: Option<&'a [u8]>,
}
#[derive(Debug)]
pub struct LowLatencyAccountUpdate {
    token_i: usize,
    last_token_i: usize,
    tokendata: Vec<u8>,
    tokenborder: Vec<u32>,
    account_i: usize,
    last_account_i: usize,
    accountdata: Vec<u8>,
    accountborder: Vec<u32>,
}
impl LowLatencyAccountUpdate {
    pub fn token_len(&self) -> usize {
        self.tokenborder.len()
    }
    pub fn account_len(&self) -> usize {
        self.accountborder.len()
    }
    pub fn token(&mut self) -> Option<&Tokenaccountv1> {
        if self.tokenborder.len() <= self.token_i {
            return None;
        }
        let finish = self.tokenborder[self.token_i] as usize;
        let start = self.last_token_i;
        self.last_token_i = finish;
        self.token_i += 1;
        let subbuf = &self.tokendata[start..finish];
        assert_eq!(std::mem::size_of::<Tokenaccountv1>(), subbuf.len());
        let ptr = subbuf.as_ptr() as *const _;
        let x: &Tokenaccountv1 = unsafe { &*ptr };
        Some(x)
    }
    pub fn account(&mut self) -> Option<AccountRef<'_>> {
        if self.accountborder.len() <= self.account_i {
            return None;
        }
        let finish = self.accountborder[self.account_i] as usize;
        let start = self.last_account_i;
        self.last_account_i = finish;
        self.account_i += 1;
        let totalbuf = &self.accountdata[start..finish];
        let header_len = std::mem::size_of::<Header>();
        assert!(header_len <= totalbuf.len());
        let header = {
            let subbuf = &totalbuf[0..header_len];
            let ptr = subbuf.as_ptr() as *const _;
            let x: &Header = unsafe { &*ptr };
            x
        };
        let body = if header_len < totalbuf.len() {
            Some(&totalbuf[header_len..])
        } else {
            None
        };
        Some(AccountRef { header, body })
    }
}
