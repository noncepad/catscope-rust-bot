use solana_sdk::pubkey::Pubkey;

use crate::{
    err::CatscopeGuestError, graph::AccountId, log_warn, message::MessageDeserializer,
    token::TokenDatabase, util::account_id_from_pubkey,
};

#[derive(Copy, Clone, Default, Debug)]
pub struct Share {
    pub token_account_id: AccountId,
    pub weight: f32,
}

pub const PORTFOLIO_SIZE: usize = 24;

#[derive(Clone, Default, Debug)]
pub struct Portfolio {
    pub size: u8,
    pub l_share: [Share; PORTFOLIO_SIZE],
}
impl MessageDeserializer for Portfolio {
    fn deserialize(&mut self, data: &[u8]) -> Result<usize, crate::err::CatscopeGuestError> {
        if data.is_empty() {
            return Err(CatscopeGuestError::InsufficientBuffer);
        }
        let mut i = 0;
        let size = data[i];
        self.size = size;
        let pubkey_len = std::mem::size_of::<Pubkey>();
        let w_len = 4;
        let share_len = pubkey_len + w_len;
        if (size as usize) % share_len != 0 {
            return Err(CatscopeGuestError::InsufficientBuffer);
        }
        let n = (size as usize) / share_len;
        for k in 0..n {
            {
                let subbuf = &data[i..(i + pubkey_len)];
                i += pubkey_len;
                let ptr = subbuf.as_ptr() as *const _;
                let pubkey: &Pubkey = unsafe { &*ptr };
                self.l_share[k].token_account_id = account_id_from_pubkey(pubkey);
            }
            {
                let subbuf = &data[i..(i + w_len)];
                i += w_len;
                let x: [u8; 4] = subbuf.try_into().unwrap();
                self.l_share[k].weight = f32::from_le_bytes(x);
            }
        }
        Ok(i)
    }
}

impl Portfolio {
    /// Number of populated entries in `l_share`, derived from `size` (a
    /// byte count) the same way `deserialize` computes it.
    fn share_count(&self) -> usize {
        let pubkey_len = std::mem::size_of::<Pubkey>();
        let share_len = pubkey_len + 4; // + weight (f32)
        (self.size as usize) / share_len
    }

    /// Sanity-check this target portfolio against what the bot currently
    /// knows about `owner`'s holdings. Logs a warning for anything that
    /// looks wrong; never panics or returns an error, since a malformed
    /// portfolio from the host shouldn't crash the bot -- just be visible
    /// in the logs.
    ///
    /// Checks: weights sum to ~1.0 (within float-rounding tolerance), no
    /// negative weight, no mint repeated across shares, and (informational
    /// only) whether the bot has seen any balance at all for a target
    /// mint yet -- not itself an error, since a brand-new position would
    /// legitimately have none.
    pub fn check(&self, owner: &AccountId, token_db: &mut TokenDatabase) {
        let n = self.share_count();
        if n == 0 {
            // Default/not-yet-initialized portfolio -- nothing to check.
            return;
        }

        let mut weight_sum = 0.0f32;
        for i in 0..n {
            let share = self.l_share[i];

            if share.weight < 0.0 {
                log_warn!(
                    "portfolio check: negative weight {} for mint {}",
                    share.weight,
                    share.token_account_id
                );
            }
            weight_sum += share.weight;

            if self.l_share[..i]
                .iter()
                .any(|s| s.token_account_id == share.token_account_id)
            {
                log_warn!(
                    "portfolio check: mint {} appears more than once in target portfolio",
                    share.token_account_id
                );
            }

            if token_db
                .balance(owner, &share.token_account_id, true)
                .is_empty()
            {
                log_warn!(
                    "portfolio check: no known balance yet for target mint {} \
                     (new position, or an unresolvable/never-seen mint)",
                    share.token_account_id
                );
            }
        }

        if (weight_sum - 1.0).abs() > 0.01 {
            log_warn!(
                "portfolio check: {} share weights sum to {} (expected ~1.0)",
                n,
                weight_sum
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catscope::witbot::shooter::Tokenaccountv1;

    const OWNER: AccountId = 1;
    const PUBKEY_LEN: usize = std::mem::size_of::<Pubkey>();
    const SHARE_LEN: usize = PUBKEY_LEN + 4;

    fn portfolio_with(shares: &[Share]) -> Portfolio {
        let mut p = Portfolio {
            size: (shares.len() * SHARE_LEN) as u8,
            ..Default::default()
        };
        for (i, s) in shares.iter().enumerate() {
            p.l_share[i] = *s;
        }
        p
    }

    fn db_with_balance(owner: AccountId, mint: AccountId, account_id: AccountId, amount: u64) -> TokenDatabase {
        let mut db = TokenDatabase::default();
        db.on_token(
            &Tokenaccountv1 {
                id: account_id,
                owner,
                mint,
                amount,
                slot: 0,
                version: 0,
            },
            true,
        );
        db
    }

    #[test]
    fn share_count_matches_size_encoding() {
        let p = portfolio_with(&[
            Share { token_account_id: 10, weight: 0.5 },
            Share { token_account_id: 20, weight: 0.5 },
        ]);
        assert_eq!(p.share_count(), 2);
    }

    #[test]
    fn empty_portfolio_is_a_no_op() {
        let p = Portfolio::default();
        let mut db = TokenDatabase::default();
        // Must not panic on a never-initialized (size == 0) portfolio.
        p.check(&OWNER, &mut db);
    }

    #[test]
    fn well_formed_portfolio_does_not_panic() {
        let p = portfolio_with(&[
            Share { token_account_id: 10, weight: 0.6 },
            Share { token_account_id: 20, weight: 0.4 },
        ]);
        let mut db = db_with_balance(OWNER, 10, 100, 5_000);
        db.on_token(
            &Tokenaccountv1 { id: 200, owner: OWNER, mint: 20, amount: 3_000, slot: 0, version: 0 },
            true,
        );
        p.check(&OWNER, &mut db);
    }

    #[test]
    fn unbalanced_weights_and_unknown_mint_do_not_panic() {
        // Weights sum to 0.5 (not ~1.0), and mint 99 has no known balance
        // at all -- both should just log, not crash.
        let p = portfolio_with(&[Share { token_account_id: 99, weight: 0.5 }]);
        let mut db = TokenDatabase::default();
        p.check(&OWNER, &mut db);
    }

    #[test]
    fn duplicate_mint_and_negative_weight_do_not_panic() {
        let p = portfolio_with(&[
            Share { token_account_id: 10, weight: -0.2 },
            Share { token_account_id: 10, weight: 1.2 },
        ]);
        let mut db = TokenDatabase::default();
        p.check(&OWNER, &mut db);
    }
}
