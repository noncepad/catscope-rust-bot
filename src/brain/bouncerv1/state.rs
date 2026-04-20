use std::{collections::VecDeque, time::SystemTime};

use solana_sdk::{
    clock::Slot,
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer as _,
    transaction::TransactionError,
};
use solana_system_interface::instruction as system_instruction;

use crate::{
    brain::bouncerv1::{
        message::Latency, Configuration, CustomMessageInbound, CustomMessageOutbound,
    },
    catscope::witbot::{
        shooter::{Header, Tokenaccountv1},
        transactionprocessor,
    },
    event::AccountWrapper,
    graph::{CommitHook, Graph, Lamports},
    log_debug, log_info,
    message::{InboundMesasgeHandler, MessageSend},
    txview::{CatscopeTransactionReadWrapper, TransactionList},
    util::pubkey_from_account_id,
    wallet::Wallet,
};

#[derive(Debug, Default)]
pub(crate) struct State {
    last_slot: Slot,
    return_wallet: Option<Pubkey>,
    do_spend: StateDoWalletSpend,
}

/// Determine whether or not there are funds that need to be sent back to the return_wallet.
#[derive(Debug, Default)]
struct StateDoWalletSpend {
    wallet_balance: Lamports,
    wallet_spent: Lamports,
    last_spend: Option<(Signature, Slot)>,
}

pub(crate) struct StateHelper<'a> {
    pub(crate) o_commit_slot: Option<Slot>,
    pub(crate) o_graph: Option<&'a mut Graph>,
    pub(crate) state: &'a mut State,
    pub(crate) configuration: &'a mut Configuration,
    pub(crate) wallet: &'a mut Wallet,
    pub(crate) q_msg: &'a mut VecDeque<MessageSend<CustomMessageOutbound>>,
}

impl<'a> StateHelper<'a> {
    pub(crate) fn on_load(&'a mut self) {
        self.configuration.count += 1;
        assert_eq!(self.configuration.count, 1);
        // do on_load stuff
        log_debug!("_+__loaded: Configuration {:?}", self.configuration);
        let keypair = Keypair::new();
        let g = self.o_graph.take().unwrap();
        let payer = self.wallet.append_key(keypair, g).unwrap();

        self.wallet.set_payer(payer);
        let pk_payer = pubkey_from_account_id(&payer).unwrap();
        log_debug!("sending wallet: pubkey {}", pk_payer);
        self.q_msg.push_back(MessageSend::Wallet(pk_payer));
        self.o_graph.replace(g);
    }

    pub(crate) fn mid_on_account(&mut self, mut account_wrapper: AccountWrapper) {
        while let Some((header, _body)) = account_wrapper.account() {
            // got account
            log_debug!("mid_on_account: {header:?}");
        }
    }

    pub(crate) fn mid_on_token(&mut self, tokenaccountv1s: Vec<Tokenaccountv1>) {
        let mut l_account_id = [0];
        for account in tokenaccountv1s.iter() {
            l_account_id[0] = account.owner;
            if let Some(header) = self.wallet.has_key(&l_account_id) {
                log_debug!("mid_on_token: {:?}; wallet {}", account, header.accountid);
            }
        }
    }
    pub(crate) fn mid_on_tx(&mut self, mut transaction_list: TransactionList) {
        while let Some((cattx, result)) = transaction_list.transaction() {
            if let Some(header) = self.wallet.has_key(cattx.account) {
                // TODO: put a real latency number here
                self.q_msg
                    .push_back(MessageSend::Custom(CustomMessageOutbound::Latency(
                        Latency { tx_first_sig: 1 },
                    )));
                match result {
                    Ok(slot) => log_debug!("mid_on_tx: result {}; old_wallet {:?}", slot, header),
                    Err(e) => log_debug!("mid_on_tx: error: {}; old_wallet {:?}", e, header),
                }
            }
        }
    }

    pub(crate) fn evaluate(&mut self) {
        let mut do_wallet = false;
        log_debug!("evaluate - 1 - do_spend {:?}", self.state.do_spend);
        let current_slot = self.state.last_slot;
        if let Some(return_wallet) = self.state.return_wallet.as_ref() {
            if self.state.do_spend.wallet_spent < self.state.do_spend.wallet_balance
                && self.state.do_spend.last_spend.is_none()
            {
                const FEE_LAMPORTS: Lamports = 5000;
                let amount = self
                    .state
                    .do_spend
                    .wallet_balance
                    .saturating_sub(self.state.do_spend.wallet_spent)
                    .saturating_sub(FEE_LAMPORTS);
                if amount > 0 {
                    let payer = self.wallet.payer_pubkey().unwrap();
                    log_debug!(
                        "evaluate - 2 - amount {amount}; payer {payer}; return {return_wallet}"
                    );
                    let ix = system_instruction::transfer(&payer, return_wallet, amount);
                    self.wallet.append_ix(ix, 0);
                    self.state.do_spend.wallet_spent += self.state.do_spend.wallet_balance;
                    do_wallet = true;
                }
            }
        }
        if let Some((signature, tx_data)) = self.wallet.assemble() {
            if do_wallet {
                log_debug!("evaluate - 3 - signature {signature}");
                transactionprocessor::send(signature.as_ref(), tx_data).expect("send transaction");
                self.state.do_spend.last_spend = Some((signature, current_slot));
            }
        }
    }

    /// only call this from CommitHook
    fn slot(&'a self) -> Slot {
        *self.o_commit_slot.as_ref().unwrap()
    }
    fn msg_send(&mut self, message: MessageSend<CustomMessageOutbound>) {
        self.q_msg.push_front(message);
    }
}

impl<'a> InboundMesasgeHandler<Configuration, CustomMessageInbound, CustomMessageOutbound>
    for StateHelper<'a>
{
    fn on_message(
        &mut self,
        action: crate::message::MessageAction<Configuration, CustomMessageInbound>,
    ) {
        match action {
            crate::message::MessageAction::PrivateKey(keypair) => {
                let pubkey = keypair.pubkey();
                let g = self.o_graph.take().unwrap();
                let account_id = self.wallet.append_key(keypair, g).unwrap();
                log_debug!("+++keypair pubkey {} {}", pubkey, account_id);
                //self.msg_send(crate::message::MessageSend::Wallet(pubkey));
            }
            crate::message::MessageAction::Ping(t) => {
                log_debug!("ping time {:?}", t);
                self.msg_send(crate::message::MessageSend::Pong(SystemTime::now()));
            }
            crate::message::MessageAction::AdjustConfiguration(new_configuration) => {
                unsafe { std::ptr::copy_nonoverlapping(&new_configuration, self.configuration, 1) };
            }
            crate::message::MessageAction::Shutdown => {
                panic!("shutting down")
            }
            crate::message::MessageAction::Custom(custom) => match custom {
                CustomMessageInbound::ReturnWallet(pubkey) => {
                    if let Some(old_pubkey) = self.state.return_wallet.replace(pubkey) {
                        log_info!("ReturnWallet replaced {old_pubkey} -> {pubkey}");
                    }
                }
                CustomMessageInbound::Blank => {
                    // do nothing
                }
            },
        }
    }

    fn message_send(&mut self, message: crate::message::MessageSend<CustomMessageOutbound>) {
        self.q_msg.push_back(message);
    }
}
impl<'a> CommitHook for StateHelper<'a> {
    fn start(&mut self, slot: Slot) {
        log_debug!("CommitHook::StateHelper - processing slot {slot}");
        assert!(self.o_commit_slot.replace(slot).is_none());
        self.state.last_slot = slot;
    }

    fn on_account(
        &mut self,
        header: &Header,
        _body: &[u8],
        _o_m_from: Option<&std::collections::HashSet<crate::graph::AccountId>>,
        _o_m_to: Option<&std::collections::HashSet<crate::graph::AccountId>>,
    ) {
        // got graph
        if self.wallet.on_account(header) {
            log_debug!(
                "wallet balance update: {:?}; do_spend {:?}",
                header,
                self.state.do_spend
            );
            self.state.do_spend.wallet_balance = header.lamports;
            if let Some((_, tx_slot)) = self.state.do_spend.last_spend.as_ref() {
                if *tx_slot <= header.slot {
                    // replace
                    self.state.do_spend.last_spend = None;
                    self.state.do_spend.wallet_spent = 0;
                }
            }
        }
    }

    fn on_token(&mut self, _token_account: &Tokenaccountv1) {
        // got graph
    }

    fn finish(&mut self) {
        self.o_commit_slot = None;
    }
}
