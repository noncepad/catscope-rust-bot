use crate::graph::EdgeManager;
use crate::log_debug;
use crate::{
    catscope::witbot::general::{self, stdin},
    err::CatscopeGuestError,
    event::{Event, EventCallback, PollEvent},
    util::rc_unlock_mut,
};
use std::{
    cell::{Cell, RefCell, UnsafeCell},
    collections::{HashMap, VecDeque},
    rc::Rc,
};

pub trait EventHandler {
    fn on_load(
        &mut self,
        poller: EventPoller,
        rc_edgemgr: Rc<UnsafeCell<EdgeManager>>,
    ) -> Result<(), CatscopeGuestError>;
    fn on_unload(&mut self) -> Result<(), CatscopeGuestError>;
    fn on_event(&mut self, event: Event) -> Result<(), CatscopeGuestError>;
    fn flush(&mut self) -> Result<(), CatscopeGuestError>;
}

pub fn run(handler: Rc<RefCell<dyn EventHandler>>) -> Result<(), CatscopeGuestError> {
    let rc_ip;
    let rc_edgemgr = Rc::new(UnsafeCell::new(EdgeManager::default()));
    {
        let is_alive = Rc::new(Cell::new(true));
        rc_ip = Rc::new(UnsafeCell::new(InnerEventPoller {
            q_event: VecDeque::with_capacity(10),
            is_alive: is_alive.clone(),
            m_event: HashMap::with_capacity(100),
        }));
        let poller = EventPoller {
            is_alive,
            inner: rc_ip.clone(),
        };
        let mut h = handler.borrow_mut();
        h.on_load(poller, rc_edgemgr.clone())?;
    }
    let ip = rc_unlock_mut(&rc_ip);
    let mut r = Ok(());

    'out1: loop {
        let p_e = general::ready();
        match PollEvent::new(p_e) {
            PollEvent::Stdin => {
                log_debug!("run - 1 - stdin");
                // this is hard coded
                let data = stdin(0);
                assert!(!data.is_empty());
                ip.q_event.push_back(Event::Stdin(data));
            }
            PollEvent::Wit(event_id) => {
                //log_debug!("run - 2 - event_id {event_id}");
                // callbacks take event wake-up signals and
                // grab data from the host.
                // then the callbacks put event messages
                // into the event queue.
                match ip.on_event_id(event_id) {
                    Ok(keep) => {
                        //log_debug!("run - 3 - event_id {event_id}; keep {keep}");
                        if !keep {
                            ip.m_event.remove(&event_id);
                        }
                    }
                    Err(e) => {
                        //log_debug!("run - 4 - event_id {event_id}; error {e}");
                        r = Err(e);
                    }
                };
                if r.is_err() {
                    break 'out1;
                }
                // grab all of the events from the event queue.
                let mut h = handler.borrow_mut();
                while let Some(event) = ip.q_event.pop_front() {
                    h.on_event(event)?;
                }
                h.flush()?;
            }
        }
    }
    ip.is_alive.set(false);
    let mut h = handler.borrow_mut();
    h.on_unload().unwrap();
    r
}

pub type IsAlive = Rc<Cell<bool>>;

#[derive(Clone)]
pub struct EventPoller {
    is_alive: IsAlive,
    inner: Rc<UnsafeCell<InnerEventPoller>>,
}
impl EventPoller {
    pub(crate) fn event(&self, event: Event) {
        let ip = rc_unlock_mut(&self.inner);
        ip.q_event.push_back(event);
    }
    pub(crate) fn register(&self, event_id: EventID, callback: Rc<UnsafeCell<dyn EventCallback>>) {
        let ip = rc_unlock_mut(&self.inner);
        assert!(ip
            .m_event
            .insert(
                event_id,
                InternalPoller {
                    id: event_id,
                    callback,
                },
            )
            .is_none());
    }
    pub(crate) fn unregister(&self, event_id: EventID) {
        let ip = rc_unlock_mut(&self.inner);
        assert!(ip.m_event.remove(&event_id,).is_some());
    }
}

pub type EventID = u32;

struct InnerEventPoller {
    is_alive: IsAlive,
    m_event: HashMap<EventID, InternalPoller>,
    q_event: VecDeque<Event>,
}
impl InnerEventPoller {
    fn on_event_id(&mut self, id: EventID) -> Result<bool, CatscopeGuestError> {
        let o_x = self.m_event.get(&id);
        if o_x.is_none() {
            return Err(CatscopeGuestError::MissingEvent(id));
        }
        let ip = o_x.unwrap();
        let callback = unsafe {
            // 1. .get() returns a raw pointer (*mut T) to the interior data
            let ptr = ip.callback.get();

            // 2. Dereference the pointer and turn it back into a mutable reference
            &mut *ptr
        };
        callback.on_event()
    }
}

struct InternalPoller {
    id: EventID,
    callback: Rc<UnsafeCell<dyn EventCallback>>,
}
