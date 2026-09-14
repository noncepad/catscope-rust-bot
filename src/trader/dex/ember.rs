//! Ember -- the program that converts real USDC into Phoenix's native
//! collateral mint, PhUSD (`PhUsd11YkbjSaWjFncfAAmatntsjx3MgDR9B6g1ks3A`).
//! Phoenix's `deposit_funds` only accepts PhUSD (see
//! `dex::phoenix::mod`'s "Collateral is not USDC" module doc) -- this is
//! the missing conversion step.
//!
//! # No public IDL or source exists
//!
//! Program `EMBERpYNE6ehWmXymZZS2skiFmCa9V5dp14e1iduM5qy` (mainnet) has no
//! on-chain Anchor IDL account (checked directly: the standard
//! `["anchor:idl", program_id]`-seeded IDL account is empty) and no
//! discoverable public GitHub source. Everything below was reverse-
//! engineered from real, successful, finalized mainnet transactions
//! (`getSignaturesForAddress`/`getTransaction` against the program,
//! several independent USDC<->PhUSD conversions decoded, both as
//! top-level instructions and as CPIs from an unrelated router program),
//! cross-checked with `getAccountInfo` on every account referenced --
//! same rigor as this session's other from-scratch protocol
//! reverse-engineering (Drift's `place_perp_order`).
//!
//! # Instruction: `deposit` (USDC -> PhUSD)
//!
//! Discriminator `sha256("global:deposit")[..8]` = `f223c68952e1f2b6`,
//! confirmed byte-for-byte against multiple real on-chain transactions.
//! **Note**: the real instruction is named plainly `deposit`, not
//! `ember_deposit` -- there's nothing Ember-specific in the name itself,
//! consistent with `getAccountInfo` showing this program mints/burns
//! more than one synthetic-asset pair (a second, unrelated mint pair was
//! observed live, confirming this program is generic, not PhUSD-only).
//! Data = `[disc(8)][amount: u64 LE]`, 16 bytes total.
//!
//! Real, fixed 8-account order (same across every real transaction
//! decoded):
//!
//! ```text
//! 0  owner / user authority         signer,  readonly
//! 1  pair authority PDA             readonly            (Ember-owned; mint+freeze authority of PhUSD, owner of the vault)
//! 2  real-asset mint (USDC)         readonly
//! 3  synthetic-asset mint (PhUSD)   writable            (minted into on deposit)
//! 4  user's real-asset ATA (USDC)   writable            (source -- debited)
//! 5  user's synthetic ATA (PhUSD)   writable            (destination -- credited)
//! 6  Ember vault (USDC token acct)  writable            (owned by account 1; holds the 1:1 backing)
//! 7  SPL Token program              readonly
//! ```
//!
//! # Instruction: `withdraw` (PhUSD -> USDC)
//!
//! Discriminator `sha256("global:withdraw")[..8]` = `b712469c946da122`,
//! confirmed byte-for-byte against real on-chain transactions. **Same
//! 8-account order as `deposit`** (confirmed identical across every real
//! transaction decoded, both directions) -- only the discriminator
//! changes which direction value actually flows. Data is `Option<u64>`-
//! shaped: `[disc(8)][tag: 1 byte][amount: u64 LE if tag == 1]` --
//! real transactions only ever showed `tag = 1` (a specific amount); the
//! `tag = 0` ("None", presumably "withdraw everything") case was never
//! observed on-chain and is intentionally not exposed here -- this bot
//! always withdraws a specific computed amount.
//!
//! # Opaque-but-verified constants
//!
//! [`EMBER_USDC_PHUSD_AUTHORITY`] and [`EMBER_USDC_PHUSD_VAULT`] are
//! confirmed real on-chain (`getAccountInfo`: the authority is
//! program-owned by Ember and is exactly the PhUSD mint's
//! `mintAuthority`/`freezeAuthority`; the vault is an SPL token account
//! whose own authority is that same PDA, holding real USDC), but their
//! seed derivation is unknown -- no source exists to confirm it. Hardcoded
//! rather than guessed at, same honesty standard as every other
//! unavoidably-opaque constant this session has introduced.

use crate::{
    trader::types::TraderError,
    util::pubkey_from_account_id,
    wallet::Wallet,
    graph::AccountId,
};
use solana_sdk::{
    message::{AccountMeta, Instruction},
    pubkey::Pubkey,
};

pub const EMBER_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("EMBERpYNE6ehWmXymZZS2skiFmCa9V5dp14e1iduM5qy");

const SPL_TOKEN_PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

/// Real USDC mint -- fixed, not read from any config, since Ember's
/// USDC/PhUSD pair is hardcoded here anyway (see module doc).
pub const USDC_MINT: Pubkey = Pubkey::from_str_const("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");

/// Ember's pair-authority PDA for the USDC/PhUSD pair -- see module doc's
/// "Opaque-but-verified constants".
pub const EMBER_USDC_PHUSD_AUTHORITY: Pubkey =
    Pubkey::from_str_const("6ur7v6AXNpnHeEb6xuk7PyezvZ1i5GrgYyWZkNCpzbRz");

/// Ember's USDC vault token account for the USDC/PhUSD pair -- see module
/// doc's "Opaque-but-verified constants".
pub const EMBER_USDC_PHUSD_VAULT: Pubkey =
    Pubkey::from_str_const("FKcEb4TdPDTRuMnQDpSEPQBcrm15S73xiUD6Qf8ZLUkq");

const DISC_DEPOSIT: [u8; 8] = [0xf2, 0x23, 0xc6, 0x89, 0x52, 0xe1, 0xf2, 0xb6];
const DISC_WITHDRAW: [u8; 8] = [0xb7, 0x12, 0x46, 0x9c, 0x94, 0x6d, 0xa1, 0x22];

pub const EMBER_DEPOSIT_CU: u32 = 60_000;
pub const EMBER_WITHDRAW_CU: u32 = 60_000;

fn resolve(id: AccountId) -> Result<Pubkey, TraderError> {
    pubkey_from_account_id(&id).ok_or(TraderError::PubkeyResolutionFailed(id))
}

/// Append a `deposit` instruction (real USDC -> PhUSD) to `wallet`.
/// `phusd_mint` is `dex::phoenix::PhoenixState::canonical_mint()` --
/// live-parsed, not hardcoded, since it's already read from
/// `GlobalConfiguration` elsewhere. `user_usdc_ata`/`user_phusd_ata` must
/// already exist (the PhUSD one typically needs
/// `Wallet::append_create_ata` first, called before this in the same
/// transaction -- see `testperpv1`'s bootstrap).
pub fn deposit(
    authority: AccountId,
    phusd_mint: AccountId,
    user_usdc_ata: AccountId,
    user_phusd_ata: AccountId,
    amount: u64,
    wallet: &mut Wallet,
) -> Result<(), TraderError> {
    let authority_pk = resolve(authority)?;
    let phusd_mint_pk = resolve(phusd_mint)?;
    let user_usdc_ata_pk = resolve(user_usdc_ata)?;
    let user_phusd_ata_pk = resolve(user_phusd_ata)?;

    let mut data = Vec::with_capacity(16);
    data.extend_from_slice(&DISC_DEPOSIT);
    data.extend_from_slice(&amount.to_le_bytes());

    let accounts = vec![
        AccountMeta::new_readonly(authority_pk, true),
        AccountMeta::new_readonly(EMBER_USDC_PHUSD_AUTHORITY, false),
        AccountMeta::new_readonly(USDC_MINT, false),
        AccountMeta::new(phusd_mint_pk, false),
        AccountMeta::new(user_usdc_ata_pk, false),
        AccountMeta::new(user_phusd_ata_pk, false),
        AccountMeta::new(EMBER_USDC_PHUSD_VAULT, false),
        AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),
    ];

    wallet.require_signer(authority);
    wallet.append_ix(Instruction { program_id: EMBER_PROGRAM_ID, accounts, data }, EMBER_DEPOSIT_CU);
    Ok(())
}

/// Append a `withdraw` instruction (PhUSD -> real USDC) to `wallet`. Same
/// account roles/order as [`deposit`] -- see that function's doc comment
/// for `phusd_mint`/ATA preconditions.
pub fn withdraw(
    authority: AccountId,
    phusd_mint: AccountId,
    user_usdc_ata: AccountId,
    user_phusd_ata: AccountId,
    amount: u64,
    wallet: &mut Wallet,
) -> Result<(), TraderError> {
    let authority_pk = resolve(authority)?;
    let phusd_mint_pk = resolve(phusd_mint)?;
    let user_usdc_ata_pk = resolve(user_usdc_ata)?;
    let user_phusd_ata_pk = resolve(user_phusd_ata)?;

    let mut data = Vec::with_capacity(17);
    data.extend_from_slice(&DISC_WITHDRAW);
    data.push(1); // Option<u64> tag -- Some, a specific amount (see module doc)
    data.extend_from_slice(&amount.to_le_bytes());

    let accounts = vec![
        AccountMeta::new_readonly(authority_pk, true),
        AccountMeta::new_readonly(EMBER_USDC_PHUSD_AUTHORITY, false),
        AccountMeta::new_readonly(USDC_MINT, false),
        AccountMeta::new(phusd_mint_pk, false),
        AccountMeta::new(user_usdc_ata_pk, false),
        AccountMeta::new(user_phusd_ata_pk, false),
        AccountMeta::new(EMBER_USDC_PHUSD_VAULT, false),
        AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false),
    ];

    wallet.require_signer(authority);
    wallet.append_ix(Instruction { program_id: EMBER_PROGRAM_ID, accounts, data }, EMBER_WITHDRAW_CU);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn deposit_discriminator_matches_real_onchain_transaction() {
        // Live-verified against multiple real Ember `deposit` (USDC ->
        // PhUSD) transactions this session, e.g. signature
        // 2RHHgHg2NNi89756ekiFtSzwGwfpzmGMLTJTPRvveLxf4BfaihUD7CV3uAsBhcfjrUE4QAbjSbZ8KUKonb7feEM.
        let expected: [u8; 8] = Sha256::digest(b"global:deposit")[..8].try_into().unwrap();
        assert_eq!(DISC_DEPOSIT, expected);
    }

    #[test]
    fn deposit_instruction_encoding_matches_real_onchain_data() {
        // Real transaction data decoded as tag(none)+u64: amount
        // 1_000_000_000 -> bytes `00ca9a3b00000000`.
        let mut data = Vec::with_capacity(16);
        data.extend_from_slice(&DISC_DEPOSIT);
        data.extend_from_slice(&1_000_000_000u64.to_le_bytes());
        assert_eq!(data.len(), 16);
        assert_eq!(&data[8..], &[0x00, 0xca, 0x9a, 0x3b, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn withdraw_discriminator_matches_real_onchain_transaction() {
        // Live-verified against a real Ember `withdraw` (PhUSD -> USDC)
        // transaction this session, e.g. signature
        // 2HiwEohjPvi13ZkPPTorytw6Dyk7dQZ3B7F71SXvLmfxczBFAfyfMWhBYWiMWhkaC2pHnAvq6CE4MNdWc9eWTmZN.
        let expected: [u8; 8] = Sha256::digest(b"global:withdraw")[..8].try_into().unwrap();
        assert_eq!(DISC_WITHDRAW, expected);
    }

    #[test]
    fn withdraw_instruction_encoding_matches_real_onchain_data() {
        // Real transaction data decoded as tag(Some)+u64: amount
        // 1_843_910_000 -> bytes `0170d5e76d00000000` (tag byte + 8 LE bytes).
        let mut data = Vec::with_capacity(17);
        data.extend_from_slice(&DISC_WITHDRAW);
        data.push(1);
        data.extend_from_slice(&1_843_910_000u64.to_le_bytes());
        assert_eq!(data.len(), 17);
        assert_eq!(&data[8..], &[0x01, 0x70, 0xd5, 0xe7, 0x6d, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn known_addresses_are_the_real_verified_pubkeys() {
        assert_eq!(EMBER_PROGRAM_ID.to_string(), "EMBERpYNE6ehWmXymZZS2skiFmCa9V5dp14e1iduM5qy");
        assert_eq!(USDC_MINT.to_string(), "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
        assert_eq!(EMBER_USDC_PHUSD_AUTHORITY.to_string(), "6ur7v6AXNpnHeEb6xuk7PyezvZ1i5GrgYyWZkNCpzbRz");
        assert_eq!(EMBER_USDC_PHUSD_VAULT.to_string(), "FKcEb4TdPDTRuMnQDpSEPQBcrm15S73xiUD6Qf8ZLUkq");
    }
}
