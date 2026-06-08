use solana_sdk::pubkey::Pubkey;

use crate::{
    err::CatscopeGuestError, graph::AccountId, message::MessageDeserializer, token::TokenDatabase,
    util::account_id_from_pubkey,
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
    pub fn check(&self, _token_db: &TokenDatabase) {}
}
