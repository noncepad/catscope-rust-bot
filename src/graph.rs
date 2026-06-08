use crate::{
    catscope::witbot::shooter::{
        self, Accountv1, Client, Commit as ShooterCommit, Header, Tokenaccountv1,
    },
    err::CatscopeGuestError,
    event::{AccountWrapper, Event, EventCallback},
    event_loop::EventPoller,
    log_warn,
    txview::TransactionList,
    util::as_bytes_mut,
};
use crate::{log_debug, log_info};
use solana_sdk::clock::Slot;
use std::collections::{HashMap, VecDeque};
use std::{
    cell::{RefCell, UnsafeCell},
    collections::HashSet,
    rc::Rc,
};

pub type AccountId = u64;
pub type Weight = u32;
pub type Depth = u8;
pub type TokenAmount = u64;
pub type Lamports = u64;

#[derive(Clone)]
pub struct Graph {
    edgemgr: Rc<RefCell<EdgeManager>>,
    poller: EventPoller,
    inner: Rc<RefCell<InnerGraph>>,
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
            edgemgr: Rc::new(RefCell::new(EdgeManager::default())),
            poller: poller.clone(),
            inner,
        };
        let g1 = Rc::new(UnsafeCell::new(g));
        poller.register(event_id, g1.clone());
        Ok(g1)
    }
    /// Add a graph subset to streaming updates.
    pub fn bulk_subscribe(
        &self,
        l_req: Vec<SubscriptionRequest>,
    ) -> Result<Vec<Subscription>, CatscopeGuestError> {
        let mx = self.inner.borrow();
        let mut l_full = Vec::with_capacity(l_req.len());
        for req in l_req {
            l_full.push((req.root, req.filter_weight, req.depth as u32));
        }
        let l_sub_id = mx.client.bulksubscribe(l_full.as_slice())?;
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
        Ok(Subscription {
            id,
            inner: self.inner.clone(),
        })
    }
}
pub struct SubscriptionRequest {
    pub root: AccountId,
    pub filter_weight: Weight,
    pub depth: Depth,
}
impl EventCallback for Graph {
    fn on_event(&mut self) -> Result<bool, CatscopeGuestError> {
        let mx = self.inner.borrow();
        let mut item = mx.client.read();
        //let l_ack = item.ack;
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

        if let Some(inner_commit) = o_commit {
            log_warn!(
                "Graph::on_event - 5 - border {}; data {}; edgeadd {}",
                inner_commit.border.len(),
                inner_commit.data.len(),
                inner_commit.edgeadd.len()
            );
            self.poller.event(Event::Commit(Commit::new(
                self.edgemgr.clone(),
                inner_commit,
            )));
        } else if let Some(slot) = o_slot {
            log_debug!("Graph::on_event - 6 - {slot}");
            let inner_commit = ShooterCommit {
                slot,
                data: vec![],
                border: vec![],
                edgeadd: vec![],
                edgeremove: vec![],
            };
            self.poller.event(Event::Commit(Commit::new(
                self.edgemgr.clone(),
                inner_commit,
            )));
        }

        //if !l_ack.is_empty() {
        //log_debug!("Graph::on_event - 7 - {l_ack:?}");
        //}

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
    rc_edgemgr: Rc<RefCell<EdgeManager>>,
    commit: ShooterCommit,
}

#[derive(Default)]
pub struct EdgeManager {
    /// from->to
    m_from: HashMap<AccountId, HashSet<AccountId>>,
    /// to -> from
    m_to: HashMap<AccountId, HashSet<AccountId>>,
}

impl Commit {
    fn new(rc_edgemgr: Rc<RefCell<EdgeManager>>, sc: ShooterCommit) -> Self {
        let mut edgemgr = rc_edgemgr.borrow_mut();
        if sc.slot % 100 == 0 {
            log_info!("shooter commit {} - 1", sc.slot);
        }
        {
            assert_eq!(sc.edgeadd.len() % 2, 0);
            let n = sc.edgeadd.len() / 2;
            for i in 0..n {
                let from = sc.edgeadd[i * 2];
                let to = sc.edgeadd[i * 2 + 1];
                {
                    let m_to = edgemgr.m_from.entry(from).or_default();
                    if !m_to.insert(to) {
                        //log_debug!("duplicate edgeadd: from={} to={}", from, to);
                    }
                }
                {
                    let m_from = edgemgr.m_to.entry(to).or_default();
                    m_from.insert(from);
                }
            }
            log_debug!("shooter commit {} - 2 - n {}", sc.slot, n);
        }
        {
            assert_eq!(sc.edgeremove.len() % 2, 0);
            let n = sc.edgeremove.len() / 2;
            for i in 0..n {
                let from = sc.edgeremove[i * 2];
                let to = sc.edgeremove[i * 2 + 1];
                {
                    if let Some(m_to) = edgemgr.m_from.get_mut(&from) {
                        if !m_to.remove(&to) {
                            //log_warn!("edgeremove: missing to={} in m_from[{}]", to, from);
                        } else if m_to.is_empty() {
                            edgemgr.m_from.remove(&from);
                        }
                    } else {
                        //log_warn!("edgeremove: missing m_from entry for from={}", from);
                    }
                }
                {
                    if let Some(m_from) = edgemgr.m_to.get_mut(&to) {
                        m_from.remove(&from);
                        if m_from.is_empty() {
                            edgemgr.m_to.remove(&to);
                        }
                    }
                }
                log_debug!("shooter commit {} - 3 - n {}", sc.slot, n);
            }
        }
        log_debug!("shooter commit {} - 4", sc.slot);
        drop(edgemgr);
        Self {
            rc_edgemgr,
            commit: sc,
        }
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
            finish = *f1 as usize;
            if finish - start == token_len {
                let subbuf = &data[start..finish];
                let dst_buf = as_bytes_mut(&mut token_account);
                dst_buf.copy_from_slice(subbuf);
                token_count += 1;
                hook.on_token(&token_account);
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

                let edgemgr = self.rc_edgemgr.borrow();
                let m_from = edgemgr.m_from.get(&header.accountid);
                let m_to = edgemgr.m_to.get(&header.accountid);
                hook.on_account(&header, d, m_from, m_to);
            } else {
                panic!("bad account length")
            }

            start = finish;
        }
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
/// Process finalized account state.
pub trait CommitHook {
    fn start(&mut self, slot: Slot);
    fn on_account(
        &mut self,
        header: &Header,
        body: &[u8],
        o_m_from: Option<&HashSet<AccountId>>,
        o_m_to: Option<&HashSet<AccountId>>,
    );
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
