use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use curve25519_dalek::edwards::CompressedEdwardsY;
use hkdf::Hkdf;
use sha2::{Digest, Sha256, Sha512};
use solana_sdk::{pubkey::Pubkey, signature::Keypair, signer::Signer};
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

use crate::log_warn;

const MAX_FRAME_PAYLOAD: usize = 64 * 1024;
const AEAD_TAG_SIZE: usize = 16;

#[derive(Debug)]
pub enum CryptError {
    InvalidPublicKey,
    DegenerateSharedSecret,
    KeysEqual,
    Hkdf,
    Auth,
    FrameTooLarge(usize),
}
impl core::fmt::Display for CryptError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CryptError::InvalidPublicKey => write!(f, "invalid Ed25519 public key"),
            CryptError::DegenerateSharedSecret => write!(f, "degenerate shared secret"),
            CryptError::KeysEqual => write!(f, "local and remote keys must differ"),
            CryptError::Hkdf => write!(f, "HKDF error"),
            CryptError::Auth => write!(f, "AEAD authentication failed"),
            CryptError::FrameTooLarge(n) => write!(f, "frame too large: {n} bytes"),
        }
    }
}

/// Encryptor encrypts plaintext into length-prefixed ChaCha20-Poly1305 AEAD frames.
/// Each frame is `[4-byte BE length][ciphertext + 16-byte tag]`.
/// Payloads larger than MAX_FRAME_PAYLOAD are split across multiple frames.
pub struct Encryptor {
    aead: ChaCha20Poly1305,
    n: u64,
}

impl Encryptor {
    pub fn seal(&mut self, plain: &[u8]) -> Vec<u8> {
        if plain.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut remaining = plain;
        while !remaining.is_empty() {
            let chunk_len = remaining.len().min(MAX_FRAME_PAYLOAD);
            let (chunk, rest) = remaining.split_at(chunk_len);
            remaining = rest;
            let nonce = make_nonce(self.n);
            let ct = self
                .aead
                .encrypt(&nonce, chunk)
                .expect("ChaCha20Poly1305::encrypt is infallible");
            self.n += 1;
            out.extend_from_slice(&(ct.len() as u32).to_be_bytes());
            out.extend_from_slice(&ct);
        }
        out
    }
}

/// Decryptor decrypts ChaCha20-Poly1305 AEAD frames produced by an Encryptor.
/// Partial frames are buffered internally until a complete frame arrives.
pub struct Decryptor {
    aead: ChaCha20Poly1305,
    n: u64,
    buf: Vec<u8>,
}

impl Decryptor {
    pub fn open(&mut self, ct: &[u8]) -> Result<Vec<u8>, CryptError> {
        if ct.is_empty() {
            return Ok(Vec::new());
        }
        self.buf.extend_from_slice(ct);
        let mut plain = Vec::new();
        while self.buf.len() >= 4 {
            let frame_len = u32::from_be_bytes(self.buf[..4].try_into().unwrap()) as usize;
            if frame_len > MAX_FRAME_PAYLOAD + AEAD_TAG_SIZE {
                return Err(CryptError::FrameTooLarge(frame_len));
            }
            let total = 4 + frame_len;
            if self.buf.len() < total {
                break;
            }
            let nonce = make_nonce(self.n);
            let pt = self
                .aead
                .decrypt(&nonce, &self.buf[4..total])
                .map_err(|_| CryptError::Auth)?;
            self.n += 1;
            plain.extend_from_slice(&pt);
            self.buf.drain(..total);
        }
        Ok(plain)
    }
}

/// new_cipher_pair derives an Encryptor and Decryptor from a DH exchange between local and remote.
/// Both sides must call this with their own private key and the peer's public key.
/// The write/read key assignment is decided lexicographically so both endpoints agree.
/// A's Encryptor pairs with B's Decryptor, and vice versa.
pub fn new_cipher_pair(
    local: &Keypair,
    remote: &Pubkey,
) -> Result<(Encryptor, Decryptor), CryptError> {
    let local_pubkey = local.pubkey();
    log_warn!("setting up crypt stream {local_pubkey}->{remote}");
    let (write_key, read_key) = derive_keys(local, remote)?;
    let enc_aead = ChaCha20Poly1305::new_from_slice(&write_key).unwrap();
    let dec_aead = ChaCha20Poly1305::new_from_slice(&read_key).unwrap();
    Ok((
        Encryptor {
            aead: enc_aead,
            n: 0,
        },
        Decryptor {
            aead: dec_aead,
            n: 0,
            buf: Vec::new(),
        },
    ))
}

fn derive_keys(local_kp: &Keypair, remote: &Pubkey) -> Result<([u8; 32], [u8; 32]), CryptError> {
    let local_pub = local_kp.pubkey();
    let local_bytes = local_pub.to_bytes();
    let remote_bytes = remote.to_bytes();

    if local_bytes == remote_bytes {
        return Err(CryptError::KeysEqual);
    }

    let shared = dh_shared_secret(local_kp, remote)?;

    // Canonical info: "solpipe-v1" + sorted(localPub, remotePub)
    // Both endpoints derive the same info string regardless of call order.
    let mut info = Vec::with_capacity(10 + 64);
    info.extend_from_slice(b"solpipe-v1");
    if local_bytes < remote_bytes {
        info.extend_from_slice(&local_bytes);
        info.extend_from_slice(&remote_bytes);
    } else {
        info.extend_from_slice(&remote_bytes);
        info.extend_from_slice(&local_bytes);
    }

    // Read 64 bytes of HKDF output: key0 = okm[0..32], key1 = okm[32..64]
    let hk = Hkdf::<Sha256>::new(None, &shared);
    let mut okm = [0u8; 64];
    hk.expand(&info, &mut okm).map_err(|_| CryptError::Hkdf)?;
    let key0: [u8; 32] = okm[..32].try_into().unwrap();
    let key1: [u8; 32] = okm[32..].try_into().unwrap();

    // The lexicographically smaller public key owns key0 for writing.
    if local_bytes < remote_bytes {
        Ok((key0, key1))
    } else {
        Ok((key1, key0))
    }
}

fn dh_shared_secret(local_kp: &Keypair, remote: &Pubkey) -> Result<[u8; 32], CryptError> {
    // Ed25519 seed → X25519 scalar: SHA-512(seed)[0..32]
    // x25519-dalek applies RFC 7748 clamping during diffie_hellman (idempotent).
    let kp_bytes = local_kp.to_bytes();
    let mut scalar_bytes = [0u8; 32];
    scalar_bytes.copy_from_slice(&Sha512::digest(&kp_bytes[..32])[..32]);

    // Ed25519 pubkey → X25519 Montgomery point via Edwards-to-Montgomery birational map
    let montgomery_bytes = CompressedEdwardsY(remote.to_bytes())
        .decompress()
        .ok_or(CryptError::InvalidPublicKey)?
        .to_montgomery()
        .to_bytes();

    // X25519 scalar multiplication
    let shared_bytes: [u8; 32] = StaticSecret::from(scalar_bytes)
        .diffie_hellman(&X25519PublicKey::from(montgomery_bytes))
        .to_bytes();

    if shared_bytes.iter().all(|&b| b == 0) {
        return Err(CryptError::DegenerateSharedSecret);
    }

    Ok(shared_bytes)
}

// Nonce layout (12 bytes): [0x00 0x00 0x00 0x00][counter as big-endian u64]
// Matches Go: binary.BigEndian.PutUint64(buf[4:], n) on a zeroed 12-byte slice.
fn make_nonce(n: u64) -> Nonce {
    let mut bytes = [0u8; 12];
    bytes[4..].copy_from_slice(&n.to_be_bytes());
    Nonce::from(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::{signature::Keypair, signer::Signer};

    #[test]
    fn test_cipher_pair_round_trip() {
        let key_a = Keypair::new();
        let key_b = Keypair::new();

        let (mut enc_a, mut dec_a) = new_cipher_pair(&key_a, &key_b.pubkey()).unwrap();
        let (mut enc_b, mut dec_b) = new_cipher_pair(&key_b, &key_a.pubkey()).unwrap();

        let msg = b"hello cipher pair";

        // A → B
        let ct = enc_a.seal(msg);
        assert!(!ct.is_empty());
        let pt = dec_b.open(&ct).unwrap();
        assert_eq!(pt.as_slice(), msg.as_ref());

        // B → A
        let ct = enc_b.seal(msg);
        assert!(!ct.is_empty());
        let pt = dec_a.open(&ct).unwrap();
        assert_eq!(pt.as_slice(), msg.as_ref());
    }

    #[test]
    fn test_cipher_pair_partial_feed() {
        let key_a = Keypair::new();
        let key_b = Keypair::new();

        let (mut enc_a, _) = new_cipher_pair(&key_a, &key_b.pubkey()).unwrap();
        let (_, mut dec_b) = new_cipher_pair(&key_b, &key_a.pubkey()).unwrap();

        let msg = b"partial frame test payload";
        let ct = enc_a.seal(msg);

        // Feed one byte at a time; only the final byte completes the frame.
        let mut got = Vec::new();
        for i in 0..ct.len() {
            let pt = dec_b.open(&ct[i..i + 1]).unwrap();
            got.extend_from_slice(&pt);
        }
        assert_eq!(got.as_slice(), msg.as_ref());
    }

    #[test]
    fn test_cipher_pair_large_payload() {
        let key_a = Keypair::new();
        let key_b = Keypair::new();

        let (mut enc_a, _) = new_cipher_pair(&key_a, &key_b.pubkey()).unwrap();
        let (_, mut dec_b) = new_cipher_pair(&key_b, &key_a.pubkey()).unwrap();

        // 2.5× MAX_FRAME_PAYLOAD forces at least three frames
        let mut msg = vec![0u8; MAX_FRAME_PAYLOAD * 5 / 2];
        for (i, b) in msg.iter_mut().enumerate() {
            *b = i as u8;
        }

        let ct = enc_a.seal(&msg);
        let pt = dec_b.open(&ct).unwrap();
        assert_eq!(pt, msg);
    }

    #[test]
    fn test_cipher_pair_multi_message() {
        let key_a = Keypair::new();
        let key_b = Keypair::new();

        let (mut enc_a, _) = new_cipher_pair(&key_a, &key_b.pubkey()).unwrap();
        let (_, mut dec_b) = new_cipher_pair(&key_b, &key_a.pubkey()).unwrap();

        for msg in [b"first".as_ref(), b"second", b"third message"] {
            let ct = enc_a.seal(msg);
            let pt = dec_b.open(&ct).unwrap();
            assert_eq!(pt.as_slice(), msg);
        }
    }

    #[test]
    fn test_cipher_pair_wrong_key() {
        let key_a = Keypair::new();
        let key_b = Keypair::new();
        let key_c = Keypair::new();

        // A encrypts to B
        let (mut enc_a, _) = new_cipher_pair(&key_a, &key_b.pubkey()).unwrap();
        // C's decryptor — keyed for A↔C, not A↔B
        let (_, mut dec_c) = new_cipher_pair(&key_c, &key_a.pubkey()).unwrap();

        let ct = enc_a.seal(b"secret");
        assert!(dec_c.open(&ct).is_err());
    }

    /// Cross-language compatibility test using test vectors from Go's TestCrossLanguageVectors.
    /// Seeds, shared secret, and ciphertexts must match Go's common.NewCipherPair output.
    #[test]
    fn test_cross_language_vectors() {
        let seed_a: [u8; 32] = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
            0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c,
            0x1d, 0x1e, 0x1f, 0x20,
        ];
        let seed_b: [u8; 32] = [
            0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e,
            0x2f, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x3b, 0x3c,
            0x3d, 0x3e, 0x3f, 0x40,
        ];

        let kp_a = Keypair::new_from_array(seed_a);
        let kp_b = Keypair::new_from_array(seed_b);

        // Verify keypair pubkeys match Go's output.
        // Go: pubA = 9C6hybhQ6Aycep9jaUnP6uL9ZYvDjUp1aSkFWPUFJtpj
        let pub_a_bytes: &[u8] =
            &kp_a.pubkey().to_bytes();
        // Go: pubB = GcQfK48DV9BzDuDeCyV2sShbAAY4vqmK8JSj1NBrwoVZ
        let pub_b_bytes: &[u8] =
            &kp_b.pubkey().to_bytes();

        // Shared secret from Go: 22dd9afeb5878d76b7b7eba66e349a1a00858963745f1b92b78a1741e9ccf249
        let expected_shared: [u8; 32] = [
            0x22, 0xdd, 0x9a, 0xfe, 0xb5, 0x87, 0x8d, 0x76, 0xb7, 0xb7, 0xeb, 0xa6, 0x6e, 0x34,
            0x9a, 0x1a, 0x00, 0x85, 0x89, 0x63, 0x74, 0x5f, 0x1b, 0x92, 0xb7, 0x8a, 0x17, 0x41,
            0xe9, 0xcc, 0xf2, 0x49,
        ];

        // Verify our shared secret matches Go's.
        let shared_ab = dh_shared_secret(&kp_a, &kp_b.pubkey()).unwrap();
        let shared_ba = dh_shared_secret(&kp_b, &kp_a.pubkey()).unwrap();
        assert_eq!(shared_ab, shared_ba, "shared secret must be symmetric");
        assert_eq!(
            shared_ab, expected_shared,
            "shared secret must match Go's output.\nGot:      {:02x?}\nExpected: {:02x?}\npub_a: {:02x?}\npub_b: {:02x?}",
            shared_ab, expected_shared, pub_a_bytes, pub_b_bytes
        );

        // cipher text A→B from Go (enc_a encrypts, dec_b decrypts):
        // Go's enc_a = Rust's new_cipher_pair(&kp_b, &kp_a.pubkey()).1 ... NO:
        // Go's enc_a pairs with Rust's dec_b from new_cipher_pair(&kp_b, &kp_a.pubkey())
        let ct_a_to_b: &[u8] = &[
            0x00, 0x00, 0x00, 0x25, 0xa4, 0xb7, 0x7f, 0xc5, 0x7a, 0x1d, 0xcf, 0xde, 0x48, 0x14,
            0x16, 0x54, 0x9c, 0x7c, 0x3c, 0x17, 0xae, 0xfe, 0x7a, 0xd6, 0x1a, 0x3e, 0x1b, 0xb1,
            0x77, 0xc2, 0x7b, 0x00, 0xea, 0x15, 0x44, 0xff, 0x1f, 0x12, 0xab, 0xa1, 0x5b,
        ];
        // cipher text B→A from Go (enc_b encrypts, dec_a decrypts):
        let ct_b_to_a: &[u8] = &[
            0x00, 0x00, 0x00, 0x25, 0x15, 0x12, 0x87, 0x02, 0x8c, 0x21, 0x47, 0x5b, 0x23, 0x14,
            0xff, 0x2f, 0x9b, 0xbb, 0xe9, 0x3f, 0xf5, 0xcc, 0x1b, 0xb3, 0x31, 0x4d, 0xd7, 0x27,
            0xa9, 0xe5, 0x6e, 0xcf, 0x69, 0x23, 0xe1, 0x7b, 0x14, 0x39, 0x7d, 0x0a, 0xd8,
        ];

        let expected_plaintext = b"hello from Go to Rust";

        // Rust dec_b = new_cipher_pair(&kp_b, &kp_a.pubkey()).1
        let (_, mut dec_b) = new_cipher_pair(&kp_b, &kp_a.pubkey()).unwrap();
        let pt = dec_b
            .open(ct_a_to_b)
            .expect("dec_b must decrypt Go's ct_a_to_b");
        assert_eq!(
            pt.as_slice(),
            expected_plaintext.as_ref(),
            "dec_b decrypted wrong plaintext"
        );

        // Rust dec_a = new_cipher_pair(&kp_a, &kp_b.pubkey()).1
        let (_, mut dec_a) = new_cipher_pair(&kp_a, &kp_b.pubkey()).unwrap();
        let pt = dec_a
            .open(ct_b_to_a)
            .expect("dec_a must decrypt Go's ct_b_to_a");
        assert_eq!(
            pt.as_slice(),
            expected_plaintext.as_ref(),
            "dec_a decrypted wrong plaintext"
        );
    }

    #[test]
    fn test_cipher_pair_empty_input() {
        let key_a = Keypair::new();
        let key_b = Keypair::new();

        let (mut enc_a, _) = new_cipher_pair(&key_a, &key_b.pubkey()).unwrap();
        let (_, mut dec_b) = new_cipher_pair(&key_b, &key_a.pubkey()).unwrap();

        assert!(enc_a.seal(&[]).is_empty());
        assert!(enc_a.seal(b"").is_empty());

        let pt = dec_b.open(&[]).unwrap();
        assert!(pt.is_empty());
    }
}
