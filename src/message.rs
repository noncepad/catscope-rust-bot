use crate::{
    crypt::{new_cipher_pair, Decryptor, Encryptor},
    err::CatscopeGuestError,
    log_debug, log_warn,
    stdio::{StdioPacket, MESSAGE_MAX_SIZE},
    util::rc_unlock,
};
use solana_sdk::{pubkey::Pubkey, signature::Keypair, signer::Signer};
use std::{
    cell::UnsafeCell,
    marker::PhantomData,
    rc::Rc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
const ENV_MOTHERSHIP_PUBKEY: &str = "MOTHERSHIP_PUBKEY";
pub struct Parser<C: Default, MIn: MessageDeserializer, MOut: MessageSerializer> {
    mothership_pubkey: Pubkey,
    keypair: Rc<UnsafeCell<Keypair>>,
    pub outbound: Option<MessageOutbound>,
    pub inbound: Option<MessageInboundParser<C, MIn>>,
    _phantom1: PhantomData<MIn>,
    _phantom2: PhantomData<MOut>,
}
impl<C: Default, MIn: MessageDeserializer, MOut: MessageSerializer> Default
    for Parser<C, MIn, MOut>
{
    fn default() -> Self {
        let mothership_pubkey = match std::env::var(ENV_MOTHERSHIP_PUBKEY) {
            Ok(x) => Pubkey::try_from(x.as_str()).expect("MOTHERSHIP_PUBKEY"),
            Err(_) => panic!("missing {ENV_MOTHERSHIP_PUBKEY} env var"),
        };
        let k = Keypair::new();
        let ephemeral_pubkey = k.pubkey();
        let (encryptor, decryptor) = new_cipher_pair(&k, &mothership_pubkey).unwrap();
        let keypair = Rc::new(UnsafeCell::new(k));
        let mut s = Self {
            mothership_pubkey,
            keypair,
            outbound: Some(MessageOutbound::new(encryptor)),
            inbound: Some(MessageInboundParser::new(decryptor)),
            _phantom1: Default::default(),
            _phantom2: Default::default(),
        };
        s.send(MessageSend::Wallet(WalleFlagEphemeral, ephemeral_pubkey));
        s
    }
}

impl<C: Default, MIn: MessageDeserializer, MOut: MessageSerializer> Parser<C, MIn, MOut> {
    pub fn send(&mut self, message: MessageSend<MOut>) {
        self.outbound.as_mut().unwrap().write(message);
    }
    pub fn mothership(&self) -> &Pubkey {
        &self.mothership_pubkey
    }
    pub fn ephemeral_key(&self) -> &Keypair {
        rc_unlock(&self.keypair)
    }
}
pub trait InboundMesasgeHandler<C: Sized + Default, MIn: MessageDeserializer, MOut> {
    fn on_message(&mut self, action: MessageAction<C, MIn>);
    fn message_send(&mut self, message: MessageSend<MOut>);
}
type WalletFlag = u8;
const WalleFlagEphemeral: WalletFlag = 0;
pub enum MessageSend<M> {
    Wallet(WalletFlag, Pubkey),
    Pong(SystemTime),
    Custom(M),
}
impl<MOut: MessageSerializer> MessageSend<MOut> {
    pub(crate) fn tag(&self) -> u8 {
        match self {
            MessageSend::Pong(_) => COMMAND_PONG,
            MessageSend::Custom(_) => COMMAND_CUSTOM,
            MessageSend::Wallet(_, _) => COMMAND_WALLET,
        }
    }
    pub(crate) fn size(&self) -> usize {
        match self {
            MessageSend::Pong(_) => 8,
            MessageSend::Wallet(_, _) => 1 + 1 + 2 + std::mem::size_of::<Pubkey>(),
            MessageSend::Custom(x) => 2 + x.len() + 20,
        }
    }
}
pub trait MessageDeserializer: Default {
    // deserialize returns the number of bytes consumed
    fn deserialize(&mut self, data: &[u8]) -> Result<usize, CatscopeGuestError>;
}

pub trait MessageSerializer {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool;
    fn serialize(&self, buffer: &mut [u8]);
}

//from bot (wasm) to optimizer(external)
const COMMAND_DUMP: u8 = 1;
const COMMAND_PONG: u8 = 2;
const COMMAND_CUSTOM: u8 = 3;
const COMMAND_WALLET: u8 = 4;
// from optimizer(external) to bot (wasm)
const COMMAND_PING: u8 = 5;
const COMMAND_SHUTDOWN: u8 = 6;

pub enum MessageAction<C: Sized + Default, MIn: MessageDeserializer> {
    Ping(SystemTime),
    Custom(MIn),
    AdjustConfiguration(C),
    Shutdown,
}

impl<'a, C: Sized + Default, MIn: MessageDeserializer> MessageAction<C, MIn> {
    pub fn parse(
        inbound: MessageInbound<'a>,
        decryptor: &'a mut Decryptor,
    ) -> Result<(Self, usize), CatscopeGuestError> {
        let size;
        let action = match inbound.cmd() {
            COMMAND_PING => {
                let d = inbound.data();
                if d.len() < 8 {
                    return Err(CatscopeGuestError::InsufficientBuffer);
                }
                let x: [u8; 8] = d[..8].try_into().unwrap();
                size = 8;
                let y = u64::from_le_bytes(x);
                let t = SystemTime::UNIX_EPOCH
                    .checked_add(Duration::from_secs(y))
                    .unwrap();
                MessageAction::Ping(t)
            }
            COMMAND_SHUTDOWN => {
                size = 0;
                MessageAction::Shutdown
            }
            COMMAND_CUSTOM => {
                let data = inbound.data();
                if data.len() < 2 {
                    return Err(CatscopeGuestError::InsufficientBuffer);
                }
                let mut i = 0;
                let n = {
                    let subbuf = &data[i..(i + 2)];
                    i += 2;
                    let y: [u8; 2] = subbuf.try_into().unwrap();
                    u16::from_le_bytes(y) as usize
                };
                if data.len() < i + n {
                    return Err(CatscopeGuestError::InsufficientBuffer);
                }
                let subbuf = &data[i..(i + n)];
                size = i + n;
                let mut body = MIn::default();
                let plaintext = decryptor.open(subbuf)?;
                let check_n = body.deserialize(&plaintext)?;
                if check_n != plaintext.len() {
                    return Err(CatscopeGuestError::InsufficientBufferV2(
                        check_n,
                        plaintext.len(),
                    ));
                }
                MessageAction::Custom(body)
            }
            _ => return Err(CatscopeGuestError::ProtocolVersionMismatch),
        };

        Ok((action, size + 1 + 1 + 4))
    }
}

pub struct MessageInboundParser<C: Sized + Default, MIn: MessageDeserializer> {
    index: usize,
    buffer: Box<[u8; MESSAGE_MAX_SIZE * 2]>,
    o_decryptor: Option<Decryptor>,
    _phantom: PhantomData<(C, MIn)>,
}

impl<C: Sized + Default, MIn: MessageDeserializer> MessageInboundParser<C, MIn> {
    pub(crate) fn new(decryptor: Decryptor) -> Self {
        Self {
            o_decryptor: Some(decryptor),
            index: 0,
            buffer: Box::new([0u8; MESSAGE_MAX_SIZE * 2]),
            _phantom: PhantomData,
        }
    }
    /// Append `data` to the internal buffer and parse as many complete messages
    /// as possible, invoking `f` for each one.  Any bytes that do not yet form
    /// a complete message are kept in the buffer for the next call.
    pub fn on_data<F>(&mut self, data: &[u8], mut f: F) -> Result<(), CatscopeGuestError>
    where
        F: FnMut(MessageAction<C, MIn>),
    {
        let end = self.index + data.len();
        log_warn!(
            "HelloWorldV1Hook::event - stdin++++++++++++++++++++++++++++++++++++++ data {}; end {}",
            data.len(),
            end
        );
        if end > self.buffer.len() {
            return Err(CatscopeGuestError::InsufficientBuffer);
        }
        self.buffer[self.index..end].copy_from_slice(data);
        self.index = end;

        let mut cursor = 0;
        loop {
            // Need at least the 6-byte header: [cmd,u8][version,u8][nonce,u32]
            if self.index - cursor < 6 {
                break;
            }

            // Pass all buffered bytes from cursor onward. MessageAction::parse
            // reports exactly how many were consumed via the returned usize.
            let (action, consumed) = {
                let slice = &self.buffer[cursor..self.index];
                let inbound = MessageInbound::read(slice)?;
                let mut decryptor = self.o_decryptor.take().unwrap();
                let r2 = MessageAction::<C, MIn>::parse(inbound, &mut decryptor);
                self.o_decryptor.replace(decryptor);
                match r2 {
                    Ok(r) => r,
                    Err(CatscopeGuestError::InsufficientBuffer) => break,
                    Err(e) => return Err(e),
                }
                // borrow of self.buffer ends here; action is fully owned
            };
            log_warn!("HelloWorldV1Hook::event - stdin++++++++++++++++++++++++++++++++++++++ action; cursor {cursor}; consumed {consumed}");

            cursor += consumed;
            f(action);
        }

        // Shift any unprocessed bytes to the front of the buffer.
        if cursor > 0 {
            self.buffer.copy_within(cursor..self.index, 0);
            self.index -= cursor;
        }

        Ok(())
    }
}

pub struct KeyValuePair<'a> {
    pub key: &'a [u8],
    pub value: &'a [u8],
}

impl<'a, 'b: 'a> TryFrom<&'b [u8]> for KeyValuePair<'a> {
    type Error = CatscopeGuestError;

    fn try_from(orig: &'b [u8]) -> Result<Self, Self::Error> {
        let mut i = 0;
        let mut subbuf = &orig[i..];
        if subbuf.is_empty() {
            return Err(CatscopeGuestError::InsufficientBuffer);
        }
        let n_k = subbuf[i] as usize;
        i += 1;
        subbuf = &orig[i..];
        if subbuf.len() < n_k {
            return Err(CatscopeGuestError::InsufficientBuffer);
        }
        let key = &orig[i..(i + n_k)];
        i += n_k;
        subbuf = &orig[i..];
        if subbuf.len() < 2 {
            return Err(CatscopeGuestError::InsufficientBuffer);
        }
        let n_v0: [u8; 2] = orig[i..(i + 2)].try_into().unwrap();
        i += 2;
        let n_v = u16::from_le_bytes(n_v0) as usize;
        subbuf = &orig[i..];
        if subbuf.len() < n_v {
            return Err(CatscopeGuestError::InsufficientBuffer);
        }
        let value = &orig[i..(i + n_v)];

        Ok(Self { key, value })
    }
}

impl<'a, 'b: 'a> MessageSerializer for KeyValuePair<'a> {
    fn len(&self) -> usize {
        1 + self.key.len() + 2 + self.value.len()
    }

    fn is_empty(&self) -> bool {
        self.key.is_empty()
    }

    fn serialize(&self, buffer: &mut [u8]) {
        let mut i = 0;
        buffer[i] = self.key.len() as u8;
        i += 1;
        {
            let subbuf = &mut buffer[i..(i + self.key.len())];
            i += self.key.len();
            subbuf.copy_from_slice(self.key);
        }
        {
            let subbuf = &mut buffer[i..(i + 2)];
            i += 2;
            let x = self.value.len() as u16;
            let y = x.to_le_bytes();
            subbuf.copy_from_slice(&y);
        }
        {
            let subbuf = &mut buffer[i..(i + self.value.len())];
            subbuf.copy_from_slice(self.value);
        }
    }
}
impl<'a, 'b: 'a> KeyValuePair<'a> {
    pub fn key(&'b self) -> &'a [u8] {
        self.key
    }
    pub fn value(&'b self) -> &'a [u8] {
        self.value
    }
}
pub struct MessageOutbound {
    encryptor: Encryptor,
    stdout: StdioPacket,
    /// increment this value by 1 for every message
    nonce: u32,
    version: u8,
}
impl MessageOutbound {
    pub(crate) fn new(encryptor: Encryptor) -> Self {
        Self {
            encryptor,
            stdout: StdioPacket::stdout(),
            nonce: 1,
            version: PROTOCOL_V1,
        }
    }
    // format: [CMD,u8][version,u8][nonce,u32]
    pub fn write<M: MessageSerializer>(&mut self, message: MessageSend<M>) {
        log_debug!("MessageOutboud::write - nonce {}", self.nonce);
        let leftover = self.stdout.buffer.len() - self.stdout.size;
        let required_buffer = 1 + 1 + 4 + message.size();
        if self.stdout.buffer.len() < required_buffer {
            panic!(
                "stdout too big: {} vs {}",
                self.stdout.buffer.len(),
                required_buffer
            )
        }

        if leftover < required_buffer {
            self.flush();
        }
        let start = self.stdout.size;
        let buffer = self.stdout.buffer.as_mut_slice();
        let wb = &mut buffer[start..];
        let mut j = 0usize;
        wb[j] = message.tag();
        j += 1;
        wb[j] = self.version;
        j += 1;
        {
            let subbuf = &mut wb[j..(j + 4)];
            j += 4;
            let x: [u8; 4] = self.nonce.to_le_bytes();
            subbuf.copy_from_slice(&x);
        }
        self.nonce += 1;
        match message {
            MessageSend::Wallet(flag, pubkey) => {
                let pubkey_len = std::mem::size_of::<Pubkey>();
                wb[j] = 1;
                j += 1;
                wb[j] = flag;
                j += 1;
                {
                    let x = pubkey_len as u16;
                    let y: [u8; 2] = x.to_le_bytes();
                    let subbuf = &mut wb[j..(j + 2)];
                    j += 2;
                    subbuf.copy_from_slice(&y);
                }
                {
                    let subbuf = &mut wb[j..(j + pubkey_len)];
                    j += pubkey_len;
                    subbuf.copy_from_slice(pubkey.as_array());
                }
            }
            MessageSend::Pong(t) => {
                let s: [u8; 8] = t
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    .to_le_bytes();
                let subbuf = &mut wb[j..(j + 8)];
                j += 8;
                subbuf.copy_from_slice(&s);
            }
            MessageSend::Custom(payload) => {
                let n = payload.len();
                let plain_start = j;
                {
                    let subbuf = &mut wb[j..];
                    payload.serialize(subbuf);
                }
                let ciphertext = self.encryptor.seal(&wb[plain_start..(plain_start + n)]);
                let cipher_n = ciphertext.len();
                {
                    let subbuf = &mut wb[j..(j + 2)];
                    j += 2;
                    let x = cipher_n as u16;
                    let y = x.to_le_bytes();
                    subbuf.copy_from_slice(&y);
                }
                {
                    let subbuf = &mut wb[j..(j + cipher_n)];
                    subbuf.copy_from_slice(&ciphertext);
                }
                j += cipher_n;
            }
        };
        self.stdout.size = start + j;
        assert_eq!(required_buffer, j);
    }

    pub fn flush(&mut self) {
        self.stdout.flush();
    }
}

pub const PROTOCOL_V1: u8 = 34;
pub struct MessageInbound<'a> {
    cmd: u8,
    nonce: u32,
    l_data: &'a [u8],
}
impl<'a, 'b: 'a> MessageInbound<'a> {
    pub(crate) fn cmd(&'b self) -> u8 {
        self.cmd
    }
    pub(crate) fn nonce(&'b self) -> u32 {
        self.nonce
    }
    pub(crate) fn data(&'b self) -> &'a [u8] {
        self.l_data
    }
    // format: [CMD,u8][version,u8][nonce,u32]
    pub(crate) fn read(data: &'b [u8]) -> Result<Self, CatscopeGuestError> {
        if data.len() < 6 {
            return Err(CatscopeGuestError::InsufficientBuffer);
        }
        let mut i = 0;
        let cmd = data[i];
        i += 1;
        let version = data[i];
        i += 1;
        if version != PROTOCOL_V1 {
            return Err(CatscopeGuestError::ProtocolVersionMismatch);
        }
        let x: [u8; 4] = data[i..(i + 4)].try_into().unwrap();
        i += 4;
        let nonce = u32::from_le_bytes(x);
        Ok(Self {
            cmd,
            nonce,
            l_data: &data[i..],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypt::new_cipher_pair;
    use solana_sdk::signature::Keypair;

    struct StringPayload(String);
    impl MessageSerializer for StringPayload {
        fn len(&self) -> usize {
            self.0.len()
        }
        fn is_empty(&self) -> bool {
            self.0.is_empty()
        }
        fn serialize(&self, buffer: &mut [u8]) {
            buffer[..self.0.len()].copy_from_slice(self.0.as_bytes());
        }
    }

    #[derive(Default)]
    struct StringDeserializer(String);
    impl MessageDeserializer for StringDeserializer {
        fn deserialize(&mut self, data: &[u8]) -> Result<usize, CatscopeGuestError> {
            self.0 = std::str::from_utf8(data).unwrap().to_string();
            Ok(data.len())
        }
    }

    #[test]
    fn test_custom_write_parse_roundtrip() {
        let key_a = Keypair::new();
        let key_b = Keypair::new();

        let (enc_a, _) = new_cipher_pair(&key_a, &key_b.pubkey()).unwrap();
        let (_, dec_b) = new_cipher_pair(&key_b, &key_a.pubkey()).unwrap();

        let payload = "hello message";
        let msg = MessageSend::<StringPayload>::Custom(StringPayload(payload.to_string()));

        let mut outbound = MessageOutbound::new(enc_a);
        outbound.write(msg);

        let written_bytes = outbound.stdout.size;
        let raw = &outbound.stdout.buffer[..written_bytes];

        let mut parser = MessageInboundParser::<(), StringDeserializer>::new(dec_b);
        let mut received = None;
        parser
            .on_data(raw, |action| {
                if let MessageAction::Custom(body) = action {
                    received = Some(body.0.clone());
                }
            })
            .unwrap();

        assert_eq!(received.as_deref(), Some(payload));
    }

    #[test]
    fn test_custom_write_parse_two_messages() {
        let key_a = Keypair::new();
        let key_b = Keypair::new();

        let (enc_a, _) = new_cipher_pair(&key_a, &key_b.pubkey()).unwrap();
        let (_, dec_b) = new_cipher_pair(&key_b, &key_a.pubkey()).unwrap();

        let mut outbound = MessageOutbound::new(enc_a);
        outbound.write(MessageSend::<StringPayload>::Custom(StringPayload(
            "first".to_string(),
        )));
        outbound.write(MessageSend::<StringPayload>::Custom(StringPayload(
            "second".to_string(),
        )));

        let written_bytes = outbound.stdout.size;
        let raw = &outbound.stdout.buffer[..written_bytes];

        let mut parser = MessageInboundParser::<(), StringDeserializer>::new(dec_b);
        let mut received = Vec::new();
        parser
            .on_data(raw, |action| {
                if let MessageAction::Custom(body) = action {
                    received.push(body.0.clone());
                }
            })
            .unwrap();

        assert_eq!(received, vec!["first", "second"]);
    }
}
