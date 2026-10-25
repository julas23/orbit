//! One-time codes as Google Authenticator makes them: TOTP (RFC 6238), an
//! HOTP (RFC 4226) of the time step, from a key given in base32 or in an
//! `otpauth://totp/…` link (the text inside the QR codes sites show).

use hmac::{Hmac, Mac};

/// The hash under the HMAC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
    Sha1,
    Sha256,
    Sha512,
}

impl Algorithm {
    pub fn name(self) -> &'static str {
        match self {
            Algorithm::Sha1 => "SHA1",
            Algorithm::Sha256 => "SHA256",
            Algorithm::Sha512 => "SHA512",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_uppercase().replace('-', "").as_str() {
            "SHA1" => Some(Algorithm::Sha1),
            "SHA256" => Some(Algorithm::Sha256),
            "SHA512" => Some(Algorithm::Sha512),
            _ => None,
        }
    }
}

/// What makes the codes of one account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Totp {
    /// The shared key, decoded.
    pub key: Vec<u8>,
    pub digits: u32,
    /// Seconds a code lasts.
    pub period: u64,
    pub algorithm: Algorithm,
    /// Who issued it ("GitHub") and the account there, from the link.
    pub issuer: String,
    pub account: String,
}

impl Totp {
    /// The usual parameters (6 digits, 30 s, SHA-1) for a base32 key.
    pub fn from_base32(key: &str) -> Option<Self> {
        let key = base32(key)?;
        (!key.is_empty()).then(|| Totp {
            key,
            digits: 6,
            period: 30,
            algorithm: Algorithm::Sha1,
            issuer: String::new(),
            account: String::new(),
        })
    }

    /// Reads `otpauth://totp/Issuer:account?secret=…&issuer=…&digits=…`.
    /// HOTP links (a counter, not the time) are refused.
    pub fn from_uri(uri: &str) -> Option<Self> {
        let rest = uri.trim().strip_prefix("otpauth://")?;
        let (kind, rest) = rest.split_once('/')?;
        if !kind.eq_ignore_ascii_case("totp") {
            return None;
        }
        let (label, query) = rest.split_once('?').unwrap_or((rest, ""));
        let label = unescape(label);
        let (mut issuer, account) = match label.split_once(':') {
            Some((issuer, account)) => (issuer.trim().to_owned(), account.trim().to_owned()),
            None => (String::new(), label.trim().to_owned()),
        };
        let mut totp = None;
        let (mut digits, mut period, mut algorithm) = (6, 30, Algorithm::Sha1);
        for pair in query.split('&') {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            let value = unescape(value);
            match name.to_ascii_lowercase().as_str() {
                "secret" => totp = Totp::from_base32(&value),
                "issuer" if !value.is_empty() => issuer = value,
                "digits" => digits = value.parse().ok().filter(|d| (6..=10).contains(d))?,
                "period" => period = value.parse().ok().filter(|p| *p > 0)?,
                "algorithm" => algorithm = Algorithm::parse(&value)?,
                _ => {}
            }
        }
        let mut totp = totp?;
        totp.digits = digits;
        totp.period = period;
        totp.algorithm = algorithm;
        totp.issuer = issuer;
        totp.account = account;
        Some(totp)
    }

    /// The code at `unix` seconds.
    pub fn code(&self, unix: u64) -> String {
        let counter = (unix / self.period).to_be_bytes();
        let digest = mac(self.algorithm, &self.key, &counter);
        // Dynamic truncation: 31 bits from the offset the last nibble gives.
        let offset = (digest[digest.len() - 1] & 0x0f) as usize;
        let value = u32::from_be_bytes(digest[offset..offset + 4].try_into().expect("4 bytes")) & 0x7fff_ffff;
        let code = value as u64 % 10u64.pow(self.digits);
        format!("{code:0width$}", width = self.digits as usize)
    }

    /// Seconds the code at `unix` still lasts.
    pub fn remaining(&self, unix: u64) -> u64 {
        self.period - unix % self.period
    }
}

fn mac(algorithm: Algorithm, key: &[u8], message: &[u8]) -> Vec<u8> {
    macro_rules! hmac {
        ($digest:ty) => {{
            let mut mac = Hmac::<$digest>::new_from_slice(key).expect("HMAC takes any key length");
            mac.update(message);
            mac.finalize().into_bytes().to_vec()
        }};
    }
    match algorithm {
        Algorithm::Sha1 => hmac!(sha1::Sha1),
        Algorithm::Sha256 => hmac!(sha2::Sha256),
        Algorithm::Sha512 => hmac!(sha2::Sha512),
    }
}

/// Base32 (RFC 4648), as the keys are written: any case, spaces and
/// dashes ignored, padding optional.
pub fn base32(text: &str) -> Option<Vec<u8>> {
    let mut bits: u64 = 0;
    let mut count = 0;
    let mut out = Vec::new();
    for c in text.chars().filter(|c| !matches!(c, ' ' | '-' | '=')) {
        let value = match c.to_ascii_uppercase() {
            c @ 'A'..='Z' => c as u64 - 'A' as u64,
            c @ '2'..='7' => c as u64 - '2' as u64 + 26,
            _ => return None,
        };
        bits = (bits << 5) | value;
        count += 5;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    Some(out)
}

/// Base32 without padding, as the key is kept.
pub fn base32_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::new();
    let (mut bits, mut count) = (0u64, 0);
    for &byte in data {
        bits = (bits << 8) | byte as u64;
        count += 8;
        while count >= 5 {
            count -= 5;
            out.push(ALPHABET[((bits >> count) & 31) as usize] as char);
        }
    }
    if count > 0 {
        out.push(ALPHABET[((bits << (5 - count)) & 31) as usize] as char);
    }
    out
}

/// Undoes the %XX escapes of a link (and + for a space, as forms write it).
fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(b) => {
                    out.push(b);
                    i += 3;
                    continue;
                }
                None => out.push(b'%'),
            },
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238, appendix B: the 20-, 32- and 64-byte keys, 8 digits.
    fn rfc(algorithm: Algorithm, key: &[u8]) -> Totp {
        Totp { key: key.to_vec(), digits: 8, period: 30, algorithm, issuer: String::new(), account: String::new() }
    }

    #[test]
    fn matches_rfc_6238() {
        let sha1 = rfc(Algorithm::Sha1, b"12345678901234567890");
        let sha256 = rfc(Algorithm::Sha256, b"12345678901234567890123456789012");
        let sha512 = rfc(Algorithm::Sha512, b"1234567890123456789012345678901234567890123456789012345678901234");
        for (time, one, two, five) in [
            (59, "94287082", "46119246", "90693936"),
            (1111111109, "07081804", "68084774", "25091201"),
            (1234567890, "89005924", "91819424", "93441116"),
            (20000000000, "65353130", "77737706", "47863826"),
        ] {
            assert_eq!(sha1.code(time), one);
            assert_eq!(sha256.code(time), two);
            assert_eq!(sha512.code(time), five);
        }
    }

    #[test]
    fn base32_round_trip() {
        assert_eq!(base32("GEZDGNBVGY3TQOJQ").unwrap(), b"1234567890");
        assert_eq!(base32("gezd gnbv-gy3t qojq====").unwrap(), b"1234567890");
        assert_eq!(base32_encode(b"1234567890"), "GEZDGNBVGY3TQOJQ");
        assert!(base32("not base32!").is_none());
    }

    #[test]
    fn reads_links() {
        let totp = Totp::from_uri(
            "otpauth://totp/ACME%20Co:john.doe@email.com?secret=HXDMVJECJJWSRB3HWIZR4IFUGFTMXBOZ&issuer=ACME%20Co&algorithm=SHA1&digits=6&period=30",
        )
        .unwrap();
        assert_eq!(totp.issuer, "ACME Co");
        assert_eq!(totp.account, "john.doe@email.com");
        assert_eq!((totp.digits, totp.period), (6, 30));
        assert!(Totp::from_uri("otpauth://hotp/x?secret=GEZDGNBV&counter=1").is_none());
        assert!(Totp::from_uri("otpauth://totp/x?issuer=nokey").is_none());
        assert_eq!(Totp::from_uri("otpauth://totp/plain?secret=GEZDGNBV").unwrap().account, "plain");
    }
}
