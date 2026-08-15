//! Value parsing shared by the evaluator and the write-time validator, so
//! both agree on what a valid number, instant, CIDR and like-pattern is.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};

/// A like-pattern shares the resource-pattern discipline: exact match, or a
/// single trailing `*` as a prefix wildcard.
pub fn is_valid_like_pattern(pattern: &str) -> bool {
    match pattern.find('*') {
        None => !pattern.is_empty(),
        Some(star) => star == pattern.len() - 1,
    }
}

pub fn like_matches(pattern: &str, candidate: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => candidate.starts_with(prefix),
        None => pattern == candidate,
    }
}

/// A numeric bound as the reference engine reads it. `None` stands for the
/// unparseable bound that makes a condition unmatchable.
pub fn parse_number(value: &str) -> Option<f64> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Some(0.0);
    }
    match trimmed {
        "Infinity" | "+Infinity" => return Some(f64::INFINITY),
        "-Infinity" => return Some(f64::NEG_INFINITY),
        _ => {}
    }
    if let Some(digits) = strip_radix_prefix(trimmed) {
        return digits;
    }
    // Rust parses spellings JavaScript rejects; refusing them keeps a policy
    // bound from meaning something different here than in the reference.
    if trimmed.eq_ignore_ascii_case("inf")
        || trimmed.eq_ignore_ascii_case("infinity")
        || trimmed.eq_ignore_ascii_case("nan")
    {
        return None;
    }
    trimmed.parse::<f64>().ok().filter(|n| !n.is_nan())
}

fn strip_radix_prefix(trimmed: &str) -> Option<Option<f64>> {
    let (digits, radix) = [("0x", 16), ("0o", 8), ("0b", 2)]
        .into_iter()
        .find_map(|(prefix, radix)| Some((strip_ci_prefix(trimmed, prefix)?, radix)))?;
    Some(
        u128::from_str_radix(digits, radix)
            .ok()
            .map(|parsed| parsed as f64),
    )
}

fn strip_ci_prefix<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let (head, rest) = value.split_at_checked(prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then_some(rest)
}

/// An instant in milliseconds since the epoch. A timestamp carrying no
/// timezone is read as UTC: the engine must decide identically wherever the
/// process runs, and `validate` refuses to store such a value anyway.
pub fn parse_instant(value: &str) -> Option<i64> {
    let trimmed = value.trim();
    // chrono accepts a leap second and folds it into the following minute; the
    // reference cannot parse one at all, so a bound naming :60 must stay
    // unmatchable rather than silently become an instant.
    if has_leap_second(trimmed) {
        return None;
    }
    if let Ok(parsed) = DateTime::parse_from_rfc3339(trimmed) {
        return Some(parsed.timestamp_millis());
    }
    if let Ok(parsed) = DateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M%#z") {
        return Some(parsed.timestamp_millis());
    }
    for format in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(trimmed, format) {
            return Some(naive.and_utc().timestamp_millis());
        }
    }
    NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
        .ok()
        .map(|date| date.and_time(chrono::NaiveTime::MIN).and_utc())
        .map(|instant: DateTime<Utc>| instant.timestamp_millis())
}

fn has_leap_second(value: &str) -> bool {
    let Some((_, time)) = value.split_once('T') else {
        return false;
    };
    let seconds = time
        .split(':')
        .nth(2)
        .map(|field| field.trim_end_matches(|c: char| !c.is_ascii_digit()));
    matches!(seconds, Some(field) if field.starts_with("60"))
}

/// The evaluation instant a caller names, so a host at the language boundary
/// never has to depend on this crate's clock library to pass one.
pub fn parse_rfc3339(raw: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(raw.trim())
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|error| format!("\"{raw}\" is not an RFC 3339 timestamp: {error}"))
}

/// A date bound must name its own timezone: a bare timestamp would mean a
/// different instant — and a different decision — on a host in another zone.
pub fn has_explicit_timezone(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.ends_with('Z') {
        return true;
    }
    // `get` rather than a slice index: the offset is a byte count, and a
    // multi-byte character straddling it would panic on a public entry point.
    [(6, true), (5, false)].into_iter().any(|(width, colon)| {
        trimmed
            .len()
            .checked_sub(width)
            .and_then(|start| trimmed.get(start..))
            .is_some_and(|tail| is_offset(tail, colon))
    })
}

fn is_offset(candidate: &str, colon: bool) -> bool {
    let bytes = candidate.as_bytes();
    let expected = if colon { 6 } else { 5 };
    if bytes.len() != expected || !matches!(bytes[0], b'+' | b'-') {
        return false;
    }
    if colon && bytes[3] != b':' {
        return false;
    }
    let digits: Vec<u8> = if colon {
        vec![bytes[1], bytes[2], bytes[4], bytes[5]]
    } else {
        vec![bytes[1], bytes[2], bytes[3], bytes[4]]
    };
    digits.iter().all(u8::is_ascii_digit)
}

pub struct Cidr {
    network: IpAddr,
    prefix_len: u32,
}

/// `<addr>/<prefixLen>` in either family, or `None` when the separator, the
/// address or the prefix length is malformed.
pub fn parse_cidr(cidr: &str) -> Option<Cidr> {
    let slash = cidr.rfind('/')?;
    let prefix_part = &cidr[slash + 1..];
    if prefix_part.is_empty()
        || prefix_part.len() > 3
        || !prefix_part.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let prefix_len: u32 = prefix_part.parse().ok()?;
    let addr_part = &cidr[..slash];
    let network = parse_ip(addr_part)?;
    let max = match network {
        IpAddr::V4(_) => 32,
        IpAddr::V6(_) => 128,
    };
    (prefix_len <= max).then_some(Cidr {
        network,
        prefix_len,
    })
}

/// IPv4-mapped IPv6 text is deliberately unsupported: one address with two
/// spellings would let a policy and a client disagree on what it names.
pub fn parse_ip(value: &str) -> Option<IpAddr> {
    if let Ok(v4) = value.parse::<Ipv4Addr>() {
        return Some(IpAddr::V4(v4));
    }
    if value.contains('.') {
        return None;
    }
    value.parse::<Ipv6Addr>().ok().map(IpAddr::V6)
}

/// `None` means unmatchable: a malformed address or CIDR, or an address
/// family mismatch — never coerced into a guess.
pub fn ip_in_cidr(ip: &str, cidr: &str) -> Option<bool> {
    let net = parse_cidr(cidr)?;
    let addr = parse_ip(ip)?;
    match (addr, net.network) {
        (IpAddr::V4(a), IpAddr::V4(b)) => {
            Some(prefix_matches(&a.octets(), &b.octets(), net.prefix_len))
        }
        (IpAddr::V6(a), IpAddr::V6(b)) => {
            Some(prefix_matches(&a.octets(), &b.octets(), net.prefix_len))
        }
        _ => None,
    }
}

fn prefix_matches(a: &[u8], b: &[u8], prefix_bits: u32) -> bool {
    let full_bytes = (prefix_bits / 8) as usize;
    if a[..full_bytes] != b[..full_bytes] {
        return false;
    }
    let remainder = prefix_bits % 8;
    if remainder == 0 {
        return true;
    }
    let mask = 0xffu8 << (8 - remainder);
    (a.get(full_bytes).copied().unwrap_or(0) & mask)
        == (b.get(full_bytes).copied().unwrap_or(0) & mask)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_octal_looking_octet_is_refused() {
        assert!(parse_ip("010.0.0.0").is_none());
        assert!(parse_cidr("010.0.0.0/8").is_none());
    }

    #[test]
    fn a_prefix_length_out_of_family_range_is_refused() {
        assert!(parse_cidr("10.0.0.0/33").is_none());
        assert!(parse_cidr("10.0.0.0").is_none());
        assert!(parse_cidr("10.0.0.0.0/8").is_none());
    }

    #[test]
    fn families_never_cross() {
        assert_eq!(ip_in_cidr("10.1.2.3", "2001:db8::/32"), None);
        assert_eq!(ip_in_cidr("2001:db8::1", "2001:db8::/32"), Some(true));
        assert_eq!(ip_in_cidr("2001:db9::1", "2001:db8::/32"), Some(false));
    }

    #[test]
    fn an_ipv4_mapped_ipv6_literal_is_refused() {
        assert!(parse_ip("::ffff:1.2.3.4").is_none());
    }

    #[test]
    fn a_zero_length_prefix_matches_everything_in_family() {
        assert_eq!(ip_in_cidr("10.1.2.3", "0.0.0.0/0"), Some(true));
        assert_eq!(ip_in_cidr("2001:db8::1", "::/0"), Some(true));
    }

    #[test]
    fn numbers_parse_as_the_reference_reads_them() {
        assert_eq!(parse_number("12"), Some(12.0));
        assert_eq!(parse_number(" 8 "), Some(8.0));
        assert_eq!(parse_number("lots"), None);
        assert_eq!(parse_number("inf"), None);
        assert_eq!(parse_number("0x10"), Some(16.0));
    }

    #[test]
    fn instants_need_a_recognisable_shape() {
        assert!(parse_instant("2026-08-01T00:00:00Z").is_some());
        assert!(parse_instant("next tuesday").is_none());
        assert!(parse_instant("2026-08-01T00:00:00Z") > parse_instant("2026-07-01T00:00:00Z"));
    }

    #[test]
    fn a_multibyte_tail_does_not_panic() {
        for candidate in ["2026-08-01T00:00:00é", "日本語テキスト", "é", "±00:00"] {
            assert!(!has_explicit_timezone(candidate));
        }
    }

    #[test]
    fn a_leap_second_is_not_an_instant() {
        assert!(parse_instant("2026-06-30T23:59:60Z").is_none());
        assert!(parse_instant("2026-06-30T23:59:60.500Z").is_none());
        assert!(parse_instant("2026-06-30T23:59:59Z").is_some());
        assert!(parse_instant("2026-06-30T00:00:06Z").is_some());
        assert!(parse_instant("2026-08-01T00:00:00+06:00").is_some());
    }

    #[test]
    fn a_bare_timestamp_carries_no_timezone() {
        assert!(has_explicit_timezone("2026-08-01T00:00:00Z"));
        assert!(has_explicit_timezone("2026-08-01T00:00:00+01:00"));
        assert!(has_explicit_timezone("2026-08-01T00:00:00+0100"));
        assert!(!has_explicit_timezone("2026-08-01T00:00:00"));
    }
}
