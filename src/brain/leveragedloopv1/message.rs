//! Stdin/stdout message serialization for leveragedloopv1.
//! Mirrors `testperpv1::message`'s shape (`Wallet`, same `KeyValuePair`
//! wire framing), plus this mode's own two manual triggers -- Phase 2 of
//! `leveraged_yield_farming_plan.md` is explicit: "manual/explicit
//! trigger only," never an automatic open. `TriggerOpen` carries the
//! starting notional (USD) at trigger time rather than baking it into
//! this bot's build-time config -- the plan flagged that number as a
//! real risk-tolerance call, decided when someone actually triggers a
//! real position, not defaulted in code. No Go-side reply this strategy
//! needs to send, so `CustomMessageOutbound` is a true no-op placeholder,
//! same as testperpv1's.
use crate::{
    err::CatscopeGuestError,
    message::{KeyValuePair, MessageDeserializer, MessageSerializer},
};
use solana_sdk::{pubkey::Pubkey, signature::Keypair, signer::Signer};
use std::{cell::UnsafeCell, rc::Rc};

pub enum CustomMessageInbound {
    Blank,
    Wallet(Rc<UnsafeCell<Keypair>>),
    /// Starting notional in USD for the first (and only, per Phase 2)
    /// loop step -- ignored unless the state machine is currently
    /// `LoopPhase::Idle`; see `on_message`'s doc comment on why a
    /// trigger that arrives mid-sequence or once a position is already
    /// open is deliberately dropped rather than queued or restarted.
    TriggerOpen(f64),
    /// Starting notional in USD, same as `TriggerOpen`, but the LST
    /// candidate is chosen automatically instead of always jitoSOL --
    /// runs the real Time-Expanded DAG
    /// (`src/trader/timegraph.rs`/`TIME.md`) over every candidate with
    /// live data (`StateHelper::dag_best_lst_path`) and opens against
    /// whichever one it picks, or declines entirely (logs, opens
    /// nothing) if the DAG itself says "do nothing" beats every real
    /// candidate right now. Same `LoopPhase::Idle`/`Closed`-only gating
    /// as `TriggerOpen`. Added 2026-08-28, Phase 3.7 of the DAG
    /// generalization plan -- `TriggerOpen`'s own behavior (always
    /// jitoSOL) is completely unchanged; this is a separate, additive
    /// trigger, not a replacement.
    TriggerOpenAuto(f64),
    /// Requests the deleverage/unwind sequence -- ignored unless the
    /// state machine is currently `LoopPhase::Open`.
    TriggerClose,
    /// One-shot recovery action, independent of `LoopPhase` (fires
    /// regardless of the loop's current phase, doesn't advance or
    /// affect it): swaps the wallet's entire real balance of the given
    /// mint back to USDC via `execute_spot_leg`. Originally added
    /// 2026-08-27 as `TriggerRecoverMsol` (hardcoded to mSOL) after a
    /// real USDC->mSOL hop succeeded but the follow-on hop never
    /// completed; genericized the same day after a *different* bug (a
    /// partial-route leak, since fixed) left the wallet holding a real,
    /// unrelated meme token (CARDS) instead -- not part of the loop's
    /// own design, a real-money recovery utility for exactly these
    /// situations, whatever mint they happen to leave behind.
    TriggerRecoverToken(Pubkey),
    /// One-shot action, independent of `LoopPhase`: swaps the given USD
    /// notional of USDC to jitoSOL and deposits it as additional Kamino
    /// collateral, mirroring `loop_borrow_and_redeposit`'s own
    /// borrow-then-redeposit pattern's second half. Added 2026-08-27
    /// after a real `BorrowAndRedeposit` cycle's automatic redeposit
    /// failed (a safe, pre-flight route rejection) and nothing ever
    /// retried it, since `already_borrowed` short-circuits straight to
    /// `Open` on every later tick -- see
    /// `redeposit_usdc_as_jitosol_collateral`'s doc comment.
    TriggerRedepositUsdc(f64),
    /// One-shot, independent of `LoopPhase`: proves the real Astralane
    /// dual-transaction bundler pipeline end-to-end with a trivial, inert
    /// real instruction (a tiny SOL self-transfer), rather than risking a
    /// real trading operation on a not-yet-live-tested mechanism. See
    /// `Wallet::send_bundler_pair`'s doc comment for what this actually
    /// exercises (durable-nonce dual-transaction fee-variant pair). No
    /// payload -- always a no-op if the durable-nonce account isn't
    /// `Ready` yet (the first trigger just bootstraps it).
    TriggerTestBundler,
    /// One-shot, independent of `LoopPhase`: proves the generic
    /// `transactionprocessor::batch` host import (the path
    /// `Wallet::drain_and_send` uses whenever more than one transaction
    /// resulted from a tick, distinct from `TriggerTestBundler`'s
    /// durable-nonce pair) with two deliberately inert self-transfer
    /// transactions. See `Wallet::test_send_two_system_transfers`'s doc
    /// comment. No payload.
    TriggerTestBatch,
    /// `(lst_mint, staking_apy)` -- real annualized SOL-per-LST
    /// exchange-rate growth for one liquid-staking token, estimated
    /// Go-side from a live timeseries this bot can't compute itself (no
    /// persistent storage across restarts) -- see
    /// `optimizer/prefetch/lst-yield`'s doc comment and
    /// `catscope-rust-bot/src/brain/leveraged_yield_farming_plan.md`'s
    /// "Phase 0". Pushed periodically (one message per real candidate in
    /// `optimizer/prefetch/lst-yield.TrackedLSTs`, currently 37), refreshed
    /// in place (see `on_message`), not accumulated. **Mint-keyed, not
    /// symbol-keyed** -- changed 2026-08-29 from the original 24-byte
    /// `(16-byte symbol, f64)` shape (still used, unchanged, by
    /// `testperpv1::message`'s identical-looking but separate variant)
    /// because a full base58 mint address doesn't fit in 16 bytes and
    /// most of the 37 real candidates don't have a confidently-known
    /// friendly name to begin with (see `LST_CANDIDATES`'s doc comment).
    /// 40-byte wire shape: 32-byte `Pubkey` + 8-byte LE `f64`, mirroring
    /// `optimizer/brain/leveragedloopv1/message.go`'s `DoLstApy`.
    LstApy(Pubkey, f64),
    /// Shared, cross-strategy: a live bundler tip update pushed by
    /// `optimizer/bundler.RunTipBroadcaster`. See
    /// `crate::bundler_message::BundlerTipUpdate`'s doc comment --
    /// consumed by `Wallet::apply_bundler_tip_update`, not this module
    /// directly.
    CommonBundlerTipUpdate(crate::bundler_message::BundlerTipUpdate),
    /// One-time opt-in for the real Phoenix-funding-vs-Kamino-rate basis
    /// trade (2026-08-29, a second, fully independent strategy from the
    /// jitoSOL leverage loop above -- see `StateHelper::run_basis_cycle`'s
    /// doc comment). No payload. Matches this bot mode's own "nothing new
    /// happens without an explicit trigger" ethos for turning a feature on
    /// in the first place -- `run_basis_cycle` itself then runs
    /// autonomously every real funding epoch once this has fired, same as
    /// `perpfundingv1`'s own real, proven behavior for this same strategy
    /// (delta-neutral by construction, not a directional leverage
    /// decision that needs a human pulling the trigger every cycle).
    TriggerEnableBasisTrading,
    /// One-shot, independent of `run_basis_cycle`'s own logic: force-closes
    /// every currently-open basis-trade position (both legs, every
    /// symbol), regardless of whether `decide_basis_trade` still agrees.
    /// No payload. A manual safety valve, mirroring `TriggerClose`'s role
    /// for the leverage loop.
    TriggerCloseAllBasisPositions,
}

impl Default for CustomMessageInbound {
    fn default() -> Self {
        Self::Blank
    }
}

const CUSTOM_KEY_FLAG_WALLET: u8 = 3;
const CUSTOM_KEY_FLAG_TRIGGER_OPEN: u8 = 4;
const CUSTOM_KEY_FLAG_TRIGGER_CLOSE: u8 = 5;
const CUSTOM_KEY_FLAG_TRIGGER_RECOVER_TOKEN: u8 = 6;
const CUSTOM_KEY_FLAG_TRIGGER_REDEPOSIT_USDC: u8 = 7;
const CUSTOM_KEY_FLAG_TRIGGER_TEST_BUNDLER: u8 = 8;
const CUSTOM_KEY_FLAG_TRIGGER_TEST_BATCH: u8 = 9;
const CUSTOM_KEY_FLAG_LST_APY: u8 = 10;
const CUSTOM_KEY_FLAG_TRIGGER_OPEN_AUTO: u8 = 11;
const CUSTOM_KEY_FLAG_TRIGGER_ENABLE_BASIS_TRADING: u8 = 12;
const CUSTOM_KEY_FLAG_TRIGGER_CLOSE_ALL_BASIS_POSITIONS: u8 = 13;

impl MessageDeserializer for CustomMessageInbound {
    fn deserialize(&mut self, body: &[u8]) -> Result<usize, CatscopeGuestError> {
        let kvp = KeyValuePair::try_from(body)?;
        let consumed = 1 + kvp.key().len() + 2 + kvp.value().len();
        let key = kvp.key();
        if key.len() != 1 {
            return Err(CatscopeGuestError::InsufficientBufferV2(key.len(), 1));
        }
        match key[0] {
            CUSTOM_KEY_FLAG_WALLET => {
                let value = kvp.value();
                if value.len() != 64 {
                    return Err(CatscopeGuestError::InsufficientBufferV2(value.len(), 64));
                }
                let secret_key = {
                    let subbuf = &value[0..32];
                    Keypair::new_from_array(subbuf.try_into().unwrap())
                };
                let pubkey = secret_key.pubkey();
                {
                    let subbuf = &value[32..];
                    let check_array = pubkey.as_array();
                    for i in 0..32 {
                        if subbuf[i] != check_array[i] {
                            return Err(CatscopeGuestError::InvalidPrivateKey);
                        }
                    }
                }
                *self = Self::Wallet(Rc::new(UnsafeCell::new(secret_key)));
            }
            CUSTOM_KEY_FLAG_TRIGGER_OPEN => {
                let value = kvp.value();
                if value.len() != 8 {
                    return Err(CatscopeGuestError::InsufficientBufferV2(value.len(), 8));
                }
                let notional_usd = f64::from_le_bytes(value.try_into().unwrap());
                *self = Self::TriggerOpen(notional_usd);
            }
            CUSTOM_KEY_FLAG_TRIGGER_OPEN_AUTO => {
                let value = kvp.value();
                if value.len() != 8 {
                    return Err(CatscopeGuestError::InsufficientBufferV2(value.len(), 8));
                }
                let notional_usd = f64::from_le_bytes(value.try_into().unwrap());
                *self = Self::TriggerOpenAuto(notional_usd);
            }
            CUSTOM_KEY_FLAG_TRIGGER_CLOSE => {
                *self = Self::TriggerClose;
            }
            CUSTOM_KEY_FLAG_TRIGGER_RECOVER_TOKEN => {
                let value = kvp.value();
                if value.len() != 32 {
                    return Err(CatscopeGuestError::InsufficientBufferV2(value.len(), 32));
                }
                let mint = Pubkey::new_from_array(value.try_into().unwrap());
                *self = Self::TriggerRecoverToken(mint);
            }
            CUSTOM_KEY_FLAG_TRIGGER_REDEPOSIT_USDC => {
                let value = kvp.value();
                if value.len() != 8 {
                    return Err(CatscopeGuestError::InsufficientBufferV2(value.len(), 8));
                }
                let notional_usd = f64::from_le_bytes(value.try_into().unwrap());
                *self = Self::TriggerRedepositUsdc(notional_usd);
            }
            CUSTOM_KEY_FLAG_TRIGGER_TEST_BUNDLER => {
                *self = Self::TriggerTestBundler;
            }
            CUSTOM_KEY_FLAG_TRIGGER_TEST_BATCH => {
                *self = Self::TriggerTestBatch;
            }
            CUSTOM_KEY_FLAG_LST_APY => {
                let value = kvp.value();
                if value.len() != 40 {
                    return Err(CatscopeGuestError::InsufficientBufferV2(value.len(), 40));
                }
                let mint = Pubkey::new_from_array(value[0..32].try_into().unwrap());
                let staking_apy = f64::from_le_bytes(value[32..40].try_into().unwrap());
                *self = Self::LstApy(mint, staking_apy);
            }
            CUSTOM_KEY_FLAG_TRIGGER_ENABLE_BASIS_TRADING => {
                *self = Self::TriggerEnableBasisTrading;
            }
            CUSTOM_KEY_FLAG_TRIGGER_CLOSE_ALL_BASIS_POSITIONS => {
                *self = Self::TriggerCloseAllBasisPositions;
            }
            crate::bundler_message::COMMON_KEY_FLAG_BUNDLER_TIP_UPDATE => {
                *self = Self::CommonBundlerTipUpdate(
                    crate::bundler_message::BundlerTipUpdate::parse(kvp.value())?,
                );
            }
            _ => {
                *self = Self::Blank;
            }
        }
        Ok(consumed)
    }
}

pub enum CustomMessageOutbound {
    Blank,
}

impl MessageSerializer for CustomMessageOutbound {
    fn len(&self) -> usize {
        0
    }
    fn is_empty(&self) -> bool {
        true
    }
    fn serialize(&self, _buffer: &mut [u8]) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire_body(key: u8, value: &[u8]) -> Vec<u8> {
        let mut buf = vec![1u8, key];
        buf.extend_from_slice(&(value.len() as u16).to_le_bytes());
        buf.extend_from_slice(value);
        buf
    }

    #[test]
    fn trigger_open_deserialize_roundtrips_notional() {
        let value = (250.0f64).to_le_bytes();
        let body = wire_body(CUSTOM_KEY_FLAG_TRIGGER_OPEN, &value);

        let mut msg = CustomMessageInbound::default();
        let consumed = msg.deserialize(&body).expect("should deserialize");

        assert_eq!(consumed, body.len());
        match msg {
            CustomMessageInbound::TriggerOpen(notional_usd) => assert_eq!(notional_usd, 250.0),
            _ => panic!("expected TriggerOpen variant"),
        }
    }

    #[test]
    fn trigger_open_deserialize_rejects_wrong_length() {
        let body = wire_body(CUSTOM_KEY_FLAG_TRIGGER_OPEN, &[0u8; 7]);
        let mut msg = CustomMessageInbound::default();
        assert!(msg.deserialize(&body).is_err());
    }

    #[test]
    fn trigger_open_auto_deserialize_roundtrips_notional() {
        let value = (75.0f64).to_le_bytes();
        let body = wire_body(CUSTOM_KEY_FLAG_TRIGGER_OPEN_AUTO, &value);

        let mut msg = CustomMessageInbound::default();
        let consumed = msg.deserialize(&body).expect("should deserialize");

        assert_eq!(consumed, body.len());
        match msg {
            CustomMessageInbound::TriggerOpenAuto(notional_usd) => assert_eq!(notional_usd, 75.0),
            _ => panic!("expected TriggerOpenAuto variant"),
        }
    }

    #[test]
    fn trigger_open_auto_deserialize_rejects_wrong_length() {
        let body = wire_body(CUSTOM_KEY_FLAG_TRIGGER_OPEN_AUTO, &[0u8; 7]);
        let mut msg = CustomMessageInbound::default();
        assert!(msg.deserialize(&body).is_err());
    }

    #[test]
    fn trigger_close_deserialize_ignores_empty_value() {
        let body = wire_body(CUSTOM_KEY_FLAG_TRIGGER_CLOSE, &[]);
        let mut msg = CustomMessageInbound::default();
        msg.deserialize(&body).expect("should deserialize");
        assert!(matches!(msg, CustomMessageInbound::TriggerClose));
    }

    #[test]
    fn trigger_test_bundler_deserialize_ignores_empty_value() {
        let body = wire_body(CUSTOM_KEY_FLAG_TRIGGER_TEST_BUNDLER, &[]);
        let mut msg = CustomMessageInbound::default();
        msg.deserialize(&body).expect("should deserialize");
        assert!(matches!(msg, CustomMessageInbound::TriggerTestBundler));
    }

    #[test]
    fn trigger_test_batch_deserialize_ignores_empty_value() {
        let body = wire_body(CUSTOM_KEY_FLAG_TRIGGER_TEST_BATCH, &[]);
        let mut msg = CustomMessageInbound::default();
        msg.deserialize(&body).expect("should deserialize");
        assert!(matches!(msg, CustomMessageInbound::TriggerTestBatch));
    }

    #[test]
    fn trigger_enable_basis_trading_deserialize_ignores_empty_value() {
        let body = wire_body(CUSTOM_KEY_FLAG_TRIGGER_ENABLE_BASIS_TRADING, &[]);
        let mut msg = CustomMessageInbound::default();
        msg.deserialize(&body).expect("should deserialize");
        assert!(matches!(msg, CustomMessageInbound::TriggerEnableBasisTrading));
    }

    #[test]
    fn trigger_close_all_basis_positions_deserialize_ignores_empty_value() {
        let body = wire_body(CUSTOM_KEY_FLAG_TRIGGER_CLOSE_ALL_BASIS_POSITIONS, &[]);
        let mut msg = CustomMessageInbound::default();
        msg.deserialize(&body).expect("should deserialize");
        assert!(matches!(msg, CustomMessageInbound::TriggerCloseAllBasisPositions));
    }

    #[test]
    fn trigger_recover_token_deserialize_roundtrips_mint() {
        let mint = Pubkey::new_unique();
        let body = wire_body(CUSTOM_KEY_FLAG_TRIGGER_RECOVER_TOKEN, mint.as_array());
        let mut msg = CustomMessageInbound::default();
        msg.deserialize(&body).expect("should deserialize");
        match msg {
            CustomMessageInbound::TriggerRecoverToken(decoded) => assert_eq!(decoded, mint),
            _ => panic!("expected TriggerRecoverToken variant"),
        }
    }

    #[test]
    fn trigger_recover_token_deserialize_rejects_wrong_length() {
        let body = wire_body(CUSTOM_KEY_FLAG_TRIGGER_RECOVER_TOKEN, &[0u8; 31]);
        let mut msg = CustomMessageInbound::default();
        assert!(msg.deserialize(&body).is_err());
    }

    #[test]
    fn trigger_redeposit_usdc_deserialize_roundtrips_notional() {
        let value = (14.93f64).to_le_bytes();
        let body = wire_body(CUSTOM_KEY_FLAG_TRIGGER_REDEPOSIT_USDC, &value);
        let mut msg = CustomMessageInbound::default();
        msg.deserialize(&body).expect("should deserialize");
        match msg {
            CustomMessageInbound::TriggerRedepositUsdc(notional_usd) => assert_eq!(notional_usd, 14.93),
            _ => panic!("expected TriggerRedepositUsdc variant"),
        }
    }

    #[test]
    fn trigger_redeposit_usdc_deserialize_rejects_wrong_length() {
        let body = wire_body(CUSTOM_KEY_FLAG_TRIGGER_REDEPOSIT_USDC, &[0u8; 7]);
        let mut msg = CustomMessageInbound::default();
        assert!(msg.deserialize(&body).is_err());
    }

    #[test]
    fn lst_apy_deserialize_roundtrips_mint_and_rate() {
        let mint = Pubkey::new_unique();
        let mut value = [0u8; 40];
        value[0..32].copy_from_slice(mint.as_array());
        value[32..40].copy_from_slice(&(0.073f64).to_le_bytes());
        let body = wire_body(CUSTOM_KEY_FLAG_LST_APY, &value);

        let mut msg = CustomMessageInbound::default();
        let consumed = msg.deserialize(&body).expect("should deserialize");

        assert_eq!(consumed, body.len());
        match msg {
            CustomMessageInbound::LstApy(decoded_mint, staking_apy) => {
                assert_eq!(decoded_mint, mint);
                assert_eq!(staking_apy, 0.073);
            }
            _ => panic!("expected LstApy variant"),
        }
    }

    #[test]
    fn lst_apy_deserialize_rejects_wrong_length() {
        let body = wire_body(CUSTOM_KEY_FLAG_LST_APY, &[0u8; 39]);
        let mut msg = CustomMessageInbound::default();
        assert!(msg.deserialize(&body).is_err());
    }

    #[test]
    fn unknown_key_flag_falls_back_to_blank() {
        let body = wire_body(0xFF, &[]);
        let mut msg = CustomMessageInbound::TriggerOpen(1.0);
        msg.deserialize(&body).expect("should deserialize");
        assert!(matches!(msg, CustomMessageInbound::Blank));
    }
}
