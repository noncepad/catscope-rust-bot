use crate::log_debug;
use crate::{
    catscope::witbot::shooter::{self, Client, Commit as ShooterCommit, Header, Tokenaccountv1},
    err::CatscopeGuestError,
    event::{AccountWrapper, Event, EventCallback},
    event_loop::EventPoller,
    log_warn,
    txview::TransactionList,
    util::as_bytes_mut,
};
use solana_sdk::clock::Slot;
use std::collections::HashMap;
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
    pub fn subscribe(
        &self,
        root: AccountId,
        filter: Weight,
        depth: Depth,
    ) -> Result<Subscription, CatscopeGuestError> {
        let mx = self.inner.borrow();
        let id = mx.client.subscribe(root, filter, depth as u32)?;
        Ok(Subscription {
            id,
            inner: self.inner.clone(),
        })
    }
}

impl EventCallback for Graph {
    fn on_event(&mut self) -> Result<bool, CatscopeGuestError> {
        //log_debug!("Graph::on_event - 1");
        let mx = self.inner.borrow();
        let (o_ack, o_slot, mut o_commit, tx_data, l_tx_border, l_token, l_account, l_sws) =
            mx.client.read();

        if !l_token.is_empty() {
            log_debug!("Graph::on_event - 2");
            self.poller.event(Event::Token(l_token));
        }

        if !l_account.is_empty() {
            log_debug!("Graph::on_event - 3");
            self.poller
                .event(Event::Account(AccountWrapper::new(l_account)));
        }
        if !tx_data.is_empty() {
            log_debug!("Graph::on_event - 4");
            self.poller.event(Event::Transaction(TransactionList::new(
                tx_data,
                l_tx_border,
            )));
        }
        if !l_sws.is_empty() {
            for sws in l_sws {
                self.poller
                    .event(Event::SlotStatus(sws.slot, sws.status.try_into().unwrap()));
            }
        }
        if let Some(inner_commit) = o_commit.take() {
            log_debug!("Graph::on_event - 5");
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
        if let Some(sub_id) = o_ack {
            log_debug!("Graph::on_event - 7 - {sub_id}");
        }

        Ok(true)
    }
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
        {
            assert_eq!(sc.edgeadd.len() % 2, 0);
            let n = sc.edgeadd.len() / 2;
            for i in 0..n {
                let from = sc.edgeadd[i];
                let to = sc.edgeadd[i + 1];
                {
                    let m_to = edgemgr.m_from.entry(from).or_default();
                    assert!(m_to.insert(to));
                }
                {
                    let m_from = edgemgr.m_to.entry(to).or_default();
                    assert!(m_from.insert(from));
                }
            }
        }
        {
            assert_eq!(sc.edgeremove.len() % 2, 0);
            let n = sc.edgeremove.len() / 2;
            for i in 0..n {
                let from = sc.edgeremove[i];
                let to = sc.edgeremove[i + 1];
                {
                    let m_to = edgemgr.m_from.get_mut(&from).unwrap();
                    assert!(m_to.remove(&to));
                }
                {
                    let m_from = edgemgr.m_to.get_mut(&to).unwrap();
                    assert!(m_from.remove(&from));
                }
            }
        }
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
            version: 0,
            lamports: 0,
            rentepoch: 0,
            accountid: 0,
            owner: 0,
            datasize: 0,
        };
        let mut token_account = Tokenaccountv1 {
            id: 0,
            owner: 0,
            mint: 0,
            amount: 0,
        };
        let mut start = 0;
        let mut finish;
        for f1 in l_border.iter() {
            finish = *f1 as usize;
            if finish - start == token_len {
                let subbuf = &data[start..finish];
                let dst_buf = as_bytes_mut(&mut token_account);
                dst_buf.copy_from_slice(subbuf);
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
                let m_to = edgemgr.m_from.get(&header.accountid);
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
