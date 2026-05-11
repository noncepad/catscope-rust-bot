use std::{
    collections::{HashSet, VecDeque},
    time::{Duration, Instant, SystemTime},
};

use solana_sdk::clock::Slot;

use crate::{
    brain::helloworldv1::{
        message::{CustomMessageInbound, CustomMessageOutbound},
        Configuration,
    },
    catscope::witbot::shooter::{Header, Tokenaccountv1},
    event::{AccountWrapper, SlotStatus},
    graph::{AccountId, CommitHook},
    log_info, log_warn,
    message::{InboundMesasgeHandler, MessageAction, MessageSend},
    txview::TransactionList,
};

#[derive(Debug)]
pub(crate) struct State {
    last_slot: Slot,
    last_print: Instant,
    pub(crate) echo: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            last_slot: 0,
            last_print: Instant::now(),
            echo: false,
        }
    }
}

pub(crate) struct StateHelper<'a> {
    pub(crate) o_commit_slot: Option<Slot>,
    pub(crate) state: &'a mut State,
    pub(crate) configuration: &'a mut Configuration,
    pub(crate) q_msg: &'a mut VecDeque<MessageSend<CustomMessageOutbound>>,
}

impl<'a> StateHelper<'a> {
    pub(crate) fn on_load(&mut self) {
        self.configuration.count += 1;
        assert_eq!(self.configuration.count, 1);
        let args: Vec<String> = std::env::args().skip(1).collect();
        self.state.echo = args.contains(&"--echo".to_string());
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
    pub(crate) fn mid_on_account(&mut self, mut account_wrapper: AccountWrapper) {
        while account_wrapper.account().is_some() {}
    }

    pub(crate) fn mid_on_token(&mut self, _tokenaccountv1s: Vec<Tokenaccountv1>) {}

    pub(crate) fn mid_on_tx(&mut self, mut transaction_list: TransactionList) {
        while transaction_list.transaction().is_some() {}
    }

    pub(crate) fn evaluate(&mut self) {
        if self.state.last_print.elapsed() >= Duration::from_secs(20) {
            log_info!("slot {}", self.state.last_slot);
            self.state.last_print = Instant::now();
        }
    }
}

impl<'a> InboundMesasgeHandler<Configuration, CustomMessageInbound, CustomMessageOutbound>
    for StateHelper<'a>
{
    fn on_message(&mut self, action: MessageAction<Configuration, CustomMessageInbound>) {
        match action {
            MessageAction::PrivateKey(_) => {}
            MessageAction::Ping(_) => {
                self.q_msg.push_back(MessageSend::Pong(SystemTime::now()));
            }
            MessageAction::AdjustConfiguration(new_configuration) => {
                unsafe { std::ptr::copy_nonoverlapping(&new_configuration, self.configuration, 1) };
            }
            MessageAction::Shutdown => panic!("shutting down"),
            MessageAction::Custom(CustomMessageInbound::EchoRequest(s)) => {
                if self.state.echo {
                    self.q_msg
                        .push_back(MessageSend::Custom(CustomMessageOutbound::EchoReply(s)));
                }
            }
            MessageAction::Custom(CustomMessageInbound::Blank) => {}
        }
    }

    fn message_send(&mut self, message: MessageSend<CustomMessageOutbound>) {
        self.q_msg.push_back(message);
    }
}

impl<'a> CommitHook for StateHelper<'a> {
    fn start(&mut self, slot: Slot) {
        assert!(self.o_commit_slot.replace(slot).is_none());
        if slot % 100 == 0 {
            log_warn!("CommitHook - start {slot}");
        }
        self.state.last_slot = slot;
    }

    fn on_account(
        &mut self,
        _header: &Header,
        _body: &[u8],
        _o_m_from: Option<&HashSet<AccountId>>,
        _o_m_to: Option<&HashSet<AccountId>>,
    ) {
    }

    fn on_token(&mut self, _token_account: &Tokenaccountv1) {}

    fn finish(&mut self) {
        self.o_commit_slot = None;
    }
}
