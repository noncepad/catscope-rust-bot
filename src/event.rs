use crate::{
    catscope::witbot::shooter::{Accountv1, Header, Tokenaccountv1},
    err::CatscopeGuestError,
    graph::Commit,
    txview::TransactionList,
};

/// Handle events from the host.
pub trait EventCallback {
    /// Get a signal from the host that it is time to read data.
    /// Read in the data and process it into an event.
    /// Send the event to the event queue in EventPoller.
    fn on_event(&mut self) -> Result<bool, CatscopeGuestError>;
}

pub enum Event {
    Stdin(Vec<u8>),
    Commit(Commit),
    Account(AccountWrapper),
    Token(Vec<Tokenaccountv1>),
    Transaction(TransactionList),
}

pub struct AccountWrapper {
    i: usize,
    list: Vec<Accountv1>,
}
impl AccountWrapper {
    pub fn new(list: Vec<Accountv1>) -> Self {
        Self { list, i: 0 }
    }
    pub fn account(&mut self) -> Option<(&Header, &[u8])> {
        if self.list.len() <= self.i {
            return None;
        }
        let x = &self.list[self.i];
        self.i += 1;
        Some((&x.header, &x.body))
    }
}
pub(crate) enum PollEvent {
    Stdin,
    Wit(u32),
}
impl PollEvent {
    pub(crate) fn new(p: (Option<u32>, Option<u32>)) -> Self {
        let (x1, x2) = p;
        Self::new2(x1, x2)
    }
    fn new2(mut o_s: Option<u32>, mut o_pe: Option<u32>) -> Self {
        assert!(o_s.is_none() || o_pe.is_none());
        if let Some(x) = o_s.take() {
            assert_eq!(x, 0);
            Self::Stdin
        } else if let Some(x) = o_pe.take() {
            Self::Wit(x)
        } else {
            panic!("have empty poll event")
        }
    }
    pub(crate) fn id(&self) -> Option<u32> {
        match self {
            PollEvent::Stdin => None,
            PollEvent::Wit(id) => Some(*id),
        }
    }
}
