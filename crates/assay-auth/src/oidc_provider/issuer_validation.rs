//! Issuer URL validation — admin-write and boot-load entry points.
//!
//! Defends against:
//!
//! - typos that store an `issuer` whose discovery endpoint hangs
//!   forever or points at internal infra;
//! - admin-CRUD-as-attack: anyone with admin write access could
//!   otherwise pin the engine's discovery client at a private-range IP
//!   to probe internal services on every login.
//!
//! [`validate_issuer`] checks the URL itself — scheme/host/userinfo/
//! fragment plus a literal-host private-range check. Gated by an
//! `allow_insecure` flag (`[auth.oidc] allow_insecure_issuers` in
//! engine config) so operators with an intentional internal IdP can
//! opt in. DNS-resolved private-range checks are intentionally not
//! performed; downstream egress controls (k8s NetworkPolicy / VPC
//! egress firewall / cloud SG) are the network-layer SSRF defence.

use std::net::IpAddr;

use url::Url;

/// Reason the issuer URL was rejected. Stringified into the admin
/// HTTP 400 error body and into `tracing::warn!` messages on the
/// boot path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IssuerError {
    /// `Url::parse` rejected the input.
    InvalidUrl(String),
    /// Scheme is neither `https` nor an opted-in `http`.
    InvalidScheme(String),
    /// Host is missing or empty.
    MissingHost,
    /// `https://user:pass@…` — would leak via `tracing::warn!`.
    HasUserinfo,
    /// `https://idp.example.com/foo#frag` — discovery URL lookups
    /// reject fragments.
    HasFragment,
    /// Host is a literal IP in a private/loopback/link-local range.
    PrivateAddress(String),
}

impl std::fmt::Display for IssuerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidUrl(e) => write!(f, "issuer is not a valid URL: {e}"),
            Self::InvalidScheme(s) => {
                write!(f, "issuer scheme {s:?} is not allowed (https required)")
            }
            Self::MissingHost => f.write_str("issuer URL has no host"),
            Self::HasUserinfo => f.write_str("issuer URL must not carry userinfo"),
            Self::HasFragment => f.write_str("issuer URL must not carry a fragment"),
            Self::PrivateAddress(host) => write!(
                f,
                "issuer host {host:?} resolves to a private/loopback/link-local address"
            ),
        }
    }
}

impl std::error::Error for IssuerError {}

/// Validate an issuer URL string. Returns the parsed `Url` on success,
/// suitable for storing back to the DB after canonicalisation.
///
/// `allow_insecure` opts into:
/// - `http://` schemes against any host;
/// - private-range literal IP hosts.
///
/// `localhost` / `127.0.0.1` / `::1` get a narrow always-on
/// `http://`-bypass (so dev rigs work without a flag), but are still
/// blocked as private-range hosts unless `allow_insecure` is set.
pub fn validate_issuer(input: &str, allow_insecure: bool) -> Result<Url, IssuerError> {
    let url = Url::parse(input).map_err(|e| IssuerError::InvalidUrl(format!("{e}")))?;
    let scheme = url.scheme();
    let host = url.host_str().ok_or(IssuerError::MissingHost)?;
    if host.is_empty() {
        return Err(IssuerError::MissingHost);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(IssuerError::HasUserinfo);
    }
    if url.fragment().is_some() {
        return Err(IssuerError::HasFragment);
    }
    match scheme {
        "https" => {}
        "http" => {
            let dev_loopback = matches!(host, "localhost" | "127.0.0.1" | "::1");
            if !allow_insecure && !dev_loopback {
                return Err(IssuerError::InvalidScheme(scheme.to_string()));
            }
        }
        other => return Err(IssuerError::InvalidScheme(other.to_string())),
    }

    if !allow_insecure
        && let Some(addr) = parse_literal_ip(host)
        && is_private_address(&addr)
    {
        return Err(IssuerError::PrivateAddress(host.to_string()));
    }
    Ok(url)
}

/// Parse `host` as a literal IP address. `Url::host_str` returns IPv6
/// addresses already-stripped, but the parser uses the bracketed form
/// in display strings; tolerate both shapes here.
fn parse_literal_ip(host: &str) -> Option<IpAddr> {
    let trimmed = host.strip_prefix('[').and_then(|s| s.strip_suffix(']'));
    let candidate = trimmed.unwrap_or(host);
    candidate.parse::<IpAddr>().ok()
}

/// Whether `addr` falls in a range we never want the discovery client
/// to talk to. Covers loopback, link-local, RFC 1918 (10/8, 172.16/12,
/// 192.168/16), RFC 6598 (100.64/10 carrier-grade NAT), 169.254/16,
/// and the IPv6 equivalents (::1, fc00::/7, fe80::/10).
pub fn is_private_address(addr: &IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_link_local()
                || v4.is_private()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || is_carrier_grade_nat(v4)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || is_unique_local_v6(v6)
                || is_link_local_v6(v6)
        }
    }
}

fn is_carrier_grade_nat(addr: &std::net::Ipv4Addr) -> bool {
    let octets = addr.octets();
    octets[0] == 100 && (64..=127).contains(&octets[1])
}

fn is_unique_local_v6(addr: &std::net::Ipv6Addr) -> bool {
    addr.segments()[0] & 0xfe00 == 0xfc00
}

fn is_link_local_v6(addr: &std::net::Ipv6Addr) -> bool {
    addr.segments()[0] & 0xffc0 == 0xfe80
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_public_host_passes() {
        let url = validate_issuer("https://accounts.google.com", false).unwrap();
        assert_eq!(url.host_str(), Some("accounts.google.com"));
    }

    #[test]
    fn https_with_path_passes() {
        let url = validate_issuer("https://idp.example.com/realms/main", false).unwrap();
        assert_eq!(url.path(), "/realms/main");
    }

    #[test]
    fn http_without_flag_rejected() {
        let err = validate_issuer("http://accounts.google.com", false).unwrap_err();
        assert!(matches!(err, IssuerError::InvalidScheme(_)));
    }

    #[test]
    fn http_with_flag_passes() {
        assert!(validate_issuer("http://accounts.google.com", true).is_ok());
    }

    #[test]
    fn http_localhost_passes_without_flag() {
        assert!(validate_issuer("http://localhost", false).is_ok());
        assert!(validate_issuer("http://127.0.0.1", false).is_err());
        assert!(validate_issuer("http://[::1]", false).is_err());
    }

    #[test]
    fn missing_host_rejected() {
        // file:// and similar schemeless URLs come out without a host;
        // explicitly check the codepath (the URL parser is lenient
        // with `https:///foo` and treats `foo` as a domain).
        let err = validate_issuer("file:///etc/passwd", false).unwrap_err();
        // Either MissingHost or InvalidScheme — both correct rejections.
        assert!(matches!(
            err,
            IssuerError::MissingHost | IssuerError::InvalidScheme(_)
        ));
    }

    #[test]
    fn userinfo_rejected() {
        let err = validate_issuer("https://user:pass@idp.example.com", false).unwrap_err();
        assert!(matches!(err, IssuerError::HasUserinfo));
    }

    #[test]
    fn fragment_rejected() {
        let err = validate_issuer("https://idp.example.com/#anchor", false).unwrap_err();
        assert!(matches!(err, IssuerError::HasFragment));
    }

    #[test]
    fn private_v4_rejected_without_flag() {
        for ip in [
            "https://192.168.1.1",
            "https://10.0.0.1",
            "https://172.16.0.1",
            "https://127.0.0.1",
            "https://169.254.1.1",
            "https://100.64.0.1",
        ] {
            let err = validate_issuer(ip, false).unwrap_err();
            assert!(
                matches!(err, IssuerError::PrivateAddress(_)),
                "{ip} should be rejected"
            );
        }
    }

    #[test]
    fn private_v6_rejected_without_flag() {
        let err = validate_issuer("https://[fc00::1]", false).unwrap_err();
        assert!(matches!(err, IssuerError::PrivateAddress(_)));
        let err = validate_issuer("https://[fe80::1]", false).unwrap_err();
        assert!(matches!(err, IssuerError::PrivateAddress(_)));
    }

    #[test]
    fn private_address_passes_with_flag() {
        assert!(validate_issuer("https://192.168.1.1", true).is_ok());
    }

    #[test]
    fn invalid_url_rejected() {
        let err = validate_issuer("not a url", false).unwrap_err();
        assert!(matches!(err, IssuerError::InvalidUrl(_)));
    }
}
