//! gcr's secret exchange, `sx-aes-1` (gcr `gcr-secret-exchange.c`): how
//! the keyring and its prompter pass a password over the session bus
//! without it being readable on the way. Each side has a Diffie-Hellman
//! key in the 1536-bit IKE group (RFC 3526); the shared secret, padded to
//! the prime's length, gives a 128-bit AES key through HKDF-SHA256 (no
//! salt, no info), and the password goes AES-128-CBC with PKCS#7 padding
//! and a random IV. Like gcr's, it does not stand up to an active
//! attacker on the bus.
//!
//! The messages are GKeyFile text:
//!
//!   [sx-aes-1]
//!   public=<base64, unsigned big-endian>
//!   secret=<base64>     (only with a password)
//!   iv=<base64>

use aes::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use gtk::glib;
use num_bigint::BigUint;
use zeroize::Zeroizing;

const PROTOCOL: &str = "sx-aes-1";

/// RFC 3526 group 5, as in gcr's `egg-dh.c`; the generator is 2.
const PRIME: [&str; 6] = [
    "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74",
    "020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F1437",
    "4FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7ED",
    "EE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF05",
    "98DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB",
    "9ED529077096966D670C354E4ABC9804F1746C08CA237327FFFFFFFFFFFFFFFF",
];
const PRIME_BYTES: usize = 192;

type Encryptor = cbc::Encryptor<aes::Aes128>;

fn prime() -> BigUint {
    BigUint::parse_bytes(PRIME.concat().as_bytes(), 16).expect("the prime")
}

/// One side's key, and the transport key once the other side's is known.
pub struct Exchange {
    private: BigUint,
    public: Vec<u8>,
    key: Option<Zeroizing<[u8; 16]>>,
}

impl Exchange {
    /// A new key pair: a random exponent below 2^1535, as gcr draws it.
    pub fn new() -> Result<Self, getrandom::Error> {
        let mut bytes = Zeroizing::new([0u8; PRIME_BYTES]);
        let private = loop {
            getrandom::getrandom(&mut bytes[..])?;
            bytes[0] &= 0x7f;
            let x = BigUint::from_bytes_be(&bytes[..]);
            if x > BigUint::from(1u8) {
                break x;
            }
        };
        let public = BigUint::from(2u8).modpow(&private, &prime()).to_bytes_be();
        Ok(Self { private, public, key: None })
    }

    /// The first message: this side's public key.
    pub fn begin(&self) -> String {
        format!("[{PROTOCOL}]\npublic={}\n", glib::base64_encode(&self.public))
    }

    /// Reads the other side's message; the first one sets the transport
    /// key. A secret it may carry is not this side's to read.
    pub fn receive(&mut self, message: &str) -> Result<(), &'static str> {
        let fields = parse(message)?;
        let peer = fields.iter().find(|(k, _)| k == "public").ok_or("no public key")?;
        let peer = glib::base64_decode(&peer.1);
        if self.key.is_some() {
            return Ok(());
        }
        let p = prime();
        let peer = BigUint::from_bytes_be(&peer);
        if peer <= BigUint::from(1u8) || peer >= &p - 1u8 {
            return Err("a public key out of range");
        }
        let shared = peer.modpow(&self.private, &p).to_bytes_be();
        // Padded to the prime's length, as gcr does.
        let mut ikm = Zeroizing::new(vec![0u8; PRIME_BYTES - shared.len()]);
        ikm.extend_from_slice(&shared);
        let mut key = Zeroizing::new([0u8; 16]);
        hkdf::Hkdf::<sha2::Sha256>::new(None, &ikm).expand(&[], &mut key[..]).map_err(|_| "HKDF")?;
        self.key = Some(key);
        Ok(())
    }

    /// The answer: this side's public key, and `secret` encrypted. Needs
    /// a message received first.
    pub fn send(&self, secret: Option<&[u8]>) -> Result<String, &'static str> {
        let mut text = self.begin();
        let Some(secret) = secret else { return Ok(text) };
        let key = self.key.as_ref().ok_or("nothing received yet")?;
        let mut iv = [0u8; 16];
        getrandom::getrandom(&mut iv).map_err(|_| "no random numbers")?;
        let sealed = Encryptor::new(key.as_ref().into(), &iv.into()).encrypt_padded_vec_mut::<Pkcs7>(secret);
        text.push_str(&format!("secret={}\niv={}\n", glib::base64_encode(&sealed), glib::base64_encode(&iv)));
        Ok(text)
    }
}

/// The `key=value` lines of the `[sx-aes-1]` group.
fn parse(message: &str) -> Result<Vec<(String, String)>, &'static str> {
    let mut lines = message.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#'));
    if lines.next() != Some(&format!("[{PROTOCOL}]")) {
        return Err("not sx-aes-1");
    }
    Ok(lines
        .take_while(|l| !l.starts_with('['))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect())
}

#[cfg(test)]
mod tests {
    use aes::cipher::BlockDecryptMut;

    use super::*;

    /// What the keyring does with the answer.
    fn open(exchange: &Exchange, message: &str) -> Vec<u8> {
        let fields = parse(message).unwrap();
        let get = |name: &str| glib::base64_decode(&fields.iter().find(|(k, _)| k == name).unwrap().1);
        let key = exchange.key.as_ref().unwrap();
        cbc::Decryptor::<aes::Aes128>::new(key.as_ref().into(), get("iv").as_slice().into())
            .decrypt_padded_vec_mut::<Pkcs7>(&get("secret"))
            .unwrap()
    }

    #[test]
    fn a_password_crosses() {
        let mut keyring = Exchange::new().unwrap();
        let mut prompter = Exchange::new().unwrap();
        prompter.receive(&keyring.begin()).unwrap();
        let answer = prompter.send(Some("correct horse".as_bytes())).unwrap();
        keyring.receive(&answer).unwrap();
        assert_eq!(open(&keyring, &answer), b"correct horse");
    }

    #[test]
    fn bad_messages_are_refused() {
        let mut e = Exchange::new().unwrap();
        assert!(e.receive("[other]\npublic=AA==\n").is_err());
        assert!(e.receive("[sx-aes-1]\n").is_err());
        assert!(e.receive("[sx-aes-1]\npublic=AQ==\n").is_err());
        assert!(e.send(Some(b"x")).is_err());
    }
}
