//! IPv4 subnets: from an address and a prefix, the network around it.
//!
//! /31 and /32 follow RFC 3021 and common use: a /31 is a point-to-point
//! link whose two addresses are both hosts, and a /32 is a single host;
//! neither has a network or broadcast address of its own.

use std::net::Ipv4Addr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    Address,
    /// A prefix above 32, or a mask whose ones are not contiguous.
    Mask,
}

/// What kind of address it is (RFC 6890 and friends).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// 10/8, 172.16/12, 192.168/16 (RFC 1918).
    Private,
    /// 100.64/10, carrier-grade NAT (RFC 6598).
    Shared,
    Loopback,
    /// 169.254/16.
    LinkLocal,
    Multicast,
    /// 0/8, 240/4 and the documentation ranges.
    Reserved,
    Broadcast,
    Public,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subnet {
    pub address: Ipv4Addr,
    pub prefix: u8,
    pub mask: Ipv4Addr,
    pub wildcard: Ipv4Addr,
    pub network: Ipv4Addr,
    /// None for /31 and /32.
    pub broadcast: Option<Ipv4Addr>,
    pub first: Ipv4Addr,
    pub last: Ipv4Addr,
    /// Usable host addresses.
    pub hosts: u64,
    /// Every address in the block.
    pub size: u64,
    /// The historical class, A to E, of the address.
    pub class: char,
    pub kind: Kind,
}

/// The mask of a prefix: 24 -> 255.255.255.0.
pub fn mask_of(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix.min(32)))
    }
}

/// The prefix of a mask, when its ones are contiguous.
pub fn prefix_of(mask: u32) -> Option<u8> {
    let ones = mask.leading_ones();
    (mask_of(ones as u8) == mask).then_some(ones as u8)
}

/// A prefix written as `24` or as a mask, `255.255.255.0`.
pub fn parse_prefix(text: &str) -> Result<u8, ParseError> {
    let text = text.trim().trim_start_matches('/');
    if text.contains('.') {
        let mask: Ipv4Addr = text.parse().map_err(|_| ParseError::Mask)?;
        prefix_of(mask.to_bits()).ok_or(ParseError::Mask)
    } else {
        match text.parse::<u8>() {
            Ok(p) if p <= 32 => Ok(p),
            _ => Err(ParseError::Mask),
        }
    }
}

/// `10.0.0.199/24`, `10.0.0.199/255.255.255.0`, `10.0.0.199 255.255.255.0`,
/// or a bare address (its prefix then `None`).
pub fn parse(text: &str) -> Result<(Ipv4Addr, Option<u8>), ParseError> {
    let text = text.trim();
    let (address, prefix) = match text.split_once(['/', ' ']) {
        Some((a, p)) => (a, Some(p)),
        None => (text, None),
    };
    let address: Ipv4Addr = address.trim().parse().map_err(|_| ParseError::Address)?;
    let prefix = prefix.map(parse_prefix).transpose()?;
    Ok((address, prefix))
}

impl Subnet {
    pub fn new(address: Ipv4Addr, prefix: u8) -> Self {
        let prefix = prefix.min(32);
        let mask = mask_of(prefix);
        let bits = address.to_bits();
        let network = bits & mask;
        let broadcast = network | !mask;
        let size = 1u64 << (32 - u32::from(prefix));
        let (first, last, hosts, has_broadcast) = match prefix {
            32 => (network, network, 1, false),
            31 => (network, broadcast, 2, false),
            _ => (network + 1, broadcast - 1, size - 2, true),
        };
        Subnet {
            address,
            prefix,
            mask: Ipv4Addr::from_bits(mask),
            wildcard: Ipv4Addr::from_bits(!mask),
            network: Ipv4Addr::from_bits(network),
            broadcast: has_broadcast.then(|| Ipv4Addr::from_bits(broadcast)),
            first: Ipv4Addr::from_bits(first),
            last: Ipv4Addr::from_bits(last),
            hosts,
            size,
            class: class(address),
            kind: kind(address),
        }
    }
}

fn class(address: Ipv4Addr) -> char {
    match address.octets()[0] {
        0..=127 => 'A',
        128..=191 => 'B',
        192..=223 => 'C',
        224..=239 => 'D',
        _ => 'E',
    }
}

fn kind(address: Ipv4Addr) -> Kind {
    let [a, b, c, _] = address.octets();
    if address.is_broadcast() {
        Kind::Broadcast
    } else if address.is_loopback() {
        Kind::Loopback
    } else if address.is_private() {
        Kind::Private
    } else if a == 100 && (64..128).contains(&b) {
        Kind::Shared
    } else if address.is_link_local() {
        Kind::LinkLocal
    } else if address.is_multicast() {
        Kind::Multicast
    } else if a == 0
        || a >= 240
        || address.is_documentation()
        || (a == 192 && b == 0 && c == 0)
        || (a == 198 && (b == 18 || b == 19))
    {
        Kind::Reserved
    } else {
        Kind::Public
    }
}

/// The address in binary, octets separated by dots and a `│` where the
/// network part ends: `00001010.00000000.00000000.│11000111` for a /24.
pub fn binary(address: Ipv4Addr, prefix: u8) -> String {
    let bits = format!("{:032b}", address.to_bits());
    let mut out = String::with_capacity(40);
    for (i, bit) in bits.chars().enumerate() {
        if i > 0 && i % 8 == 0 {
            out.push('.');
        }
        if i == usize::from(prefix) && i > 0 {
            out.push('│');
        }
        out.push(bit);
    }
    out
}

/// The address in hexadecimal, by octet: `0A.00.00.C7`.
pub fn hex(address: Ipv4Addr) -> String {
    let [a, b, c, d] = address.octets();
    format!("{a:02X}.{b:02X}.{c:02X}.{d:02X}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    #[test]
    fn a_slash_24() {
        let s = Subnet::new(ip("10.0.0.199"), 24);
        assert_eq!(s.mask, ip("255.255.255.0"));
        assert_eq!(s.wildcard, ip("0.0.0.255"));
        assert_eq!(s.network, ip("10.0.0.0"));
        assert_eq!(s.broadcast, Some(ip("10.0.0.255")));
        assert_eq!(s.first, ip("10.0.0.1"));
        assert_eq!(s.last, ip("10.0.0.254"));
        assert_eq!((s.hosts, s.size), (254, 256));
        assert_eq!((s.class, s.kind), ('A', Kind::Private));
    }

    #[test]
    fn odd_prefixes() {
        let s = Subnet::new(ip("172.16.5.130"), 26);
        assert_eq!(s.network, ip("172.16.5.128"));
        assert_eq!(s.broadcast, Some(ip("172.16.5.191")));
        assert_eq!((s.first, s.last), (ip("172.16.5.129"), ip("172.16.5.190")));
        assert_eq!(s.hosts, 62);

        let s = Subnet::new(ip("192.168.1.7"), 31);
        assert_eq!(
            (s.first, s.last, s.broadcast),
            (ip("192.168.1.6"), ip("192.168.1.7"), None)
        );
        assert_eq!(s.hosts, 2);

        let s = Subnet::new(ip("8.8.8.8"), 32);
        assert_eq!(
            (s.first, s.last, s.hosts),
            (ip("8.8.8.8"), ip("8.8.8.8"), 1)
        );
        assert_eq!(s.kind, Kind::Public);

        let s = Subnet::new(ip("1.2.3.4"), 0);
        assert_eq!(s.network, ip("0.0.0.0"));
        assert_eq!(s.broadcast, Some(ip("255.255.255.255")));
        assert_eq!((s.size, s.hosts), (1 << 32, (1 << 32) - 2));
    }

    #[test]
    fn parsing() {
        assert_eq!(parse("10.0.0.199/24"), Ok((ip("10.0.0.199"), Some(24))));
        assert_eq!(
            parse(" 10.0.0.199/255.255.252.0 "),
            Ok((ip("10.0.0.199"), Some(22)))
        );
        assert_eq!(
            parse("10.0.0.199 255.255.255.128"),
            Ok((ip("10.0.0.199"), Some(25)))
        );
        assert_eq!(parse("10.0.0.199"), Ok((ip("10.0.0.199"), None)));
        assert_eq!(parse("10.0.0/24"), Err(ParseError::Address));
        assert_eq!(parse("10.0.0.1/33"), Err(ParseError::Mask));
        assert_eq!(parse("10.0.0.1/255.0.255.0"), Err(ParseError::Mask));
        assert_eq!(parse_prefix("/16"), Ok(16));
        assert_eq!(prefix_of(0), Some(0));
        assert_eq!(prefix_of(u32::MAX), Some(32));
    }

    #[test]
    fn kinds_and_classes() {
        assert_eq!(kind(ip("127.0.0.1")), Kind::Loopback);
        assert_eq!(kind(ip("100.100.1.1")), Kind::Shared);
        assert_eq!(kind(ip("169.254.3.3")), Kind::LinkLocal);
        assert_eq!(kind(ip("224.0.0.251")), Kind::Multicast);
        assert_eq!(kind(ip("192.0.2.10")), Kind::Reserved);
        assert_eq!(kind(ip("255.255.255.255")), Kind::Broadcast);
        assert_eq!(class(ip("150.1.1.1")), 'B');
        assert_eq!(class(ip("230.1.1.1")), 'D');
    }

    #[test]
    fn hex_form() {
        assert_eq!(hex(ip("10.0.0.199")), "0A.00.00.C7");
        assert_eq!(hex(ip("255.255.255.0")), "FF.FF.FF.00");
    }

    #[test]
    fn binary_form() {
        assert_eq!(
            binary(ip("10.0.0.199"), 24),
            "00001010.00000000.00000000.│11000111"
        );
        assert_eq!(
            binary(ip("172.16.5.130"), 26),
            "10101100.00010000.00000101.10│000010"
        );
        assert_eq!(
            binary(ip("1.1.1.1"), 32),
            "00000001.00000001.00000001.00000001"
        );
    }
}
