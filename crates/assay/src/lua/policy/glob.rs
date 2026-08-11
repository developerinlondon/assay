//! Host and path matching for policy rules.
//!
//! Hosts take a single leading-wildcard form rather than a general glob:
//! `*example.com` would match `evilexample.com`, which is a footgun in an
//! allowlist. Paths use `*` (one segment) and `**` (across segments).

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Lit(String),
    Star,
    DoubleStar,
}

/// `*` matches any host. `*.example.com` matches any strict subdomain but
/// not the apex. Everything else is an exact, case-insensitive match.
pub fn host_matches(pattern: &str, host: &str) -> bool {
    let pattern = pattern.trim().to_ascii_lowercase();
    let host = host.trim().to_ascii_lowercase();
    if pattern == "*" {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix("*.") {
        return host
            .strip_suffix(suffix)
            .is_some_and(|head| head.len() > 1 && head.ends_with('.'));
    }
    pattern == host
}

/// `*` matches any run of characters except `/`; `**` matches any run
/// including `/`. Anchored at both ends, so `/v3` does not match `/v3/auth`.
pub fn path_matches(pattern: &str, path: &str) -> bool {
    match_toks(&tokenize(pattern), path)
}

fn tokenize(pattern: &str) -> Vec<Tok> {
    let mut toks = Vec::new();
    let mut lit = String::new();
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '*' {
            lit.push(c);
            continue;
        }
        if !lit.is_empty() {
            toks.push(Tok::Lit(std::mem::take(&mut lit)));
        }
        if chars.peek() == Some(&'*') {
            chars.next();
            toks.push(Tok::DoubleStar);
        } else {
            toks.push(Tok::Star);
        }
    }
    if !lit.is_empty() {
        toks.push(Tok::Lit(lit));
    }
    toks
}

fn match_toks(toks: &[Tok], s: &str) -> bool {
    let Some(head) = toks.first() else {
        return s.is_empty();
    };
    let rest = &toks[1..];
    match head {
        Tok::Lit(lit) => match s.strip_prefix(lit.as_str()) {
            Some(tail) => match_toks(rest, tail),
            None => false,
        },
        Tok::Star => {
            let limit = s.find('/').unwrap_or(s.len());
            (0..=limit).any(|i| s.is_char_boundary(i) && match_toks(rest, &s[i..]))
        }
        Tok::DoubleStar => {
            (0..=s.len()).any(|i| s.is_char_boundary(i) && match_toks(rest, &s[i..]))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_wildcard_matches_subdomains_but_not_apex() {
        assert!(host_matches("*.example.com", "identity.example.com"));
        assert!(host_matches("*.example.com", "a.b.example.com"));
        assert!(!host_matches("*.example.com", "example.com"));
    }

    #[test]
    fn host_wildcard_does_not_match_a_lookalike_suffix() {
        assert!(!host_matches("*.example.com", "evilexample.com"));
        assert!(!host_matches("example.com", "evil-example.com"));
    }

    #[test]
    fn host_bare_star_matches_everything_and_exact_is_case_insensitive() {
        assert!(host_matches("*", "anything.internal"));
        assert!(host_matches("Example.COM", "example.com"));
    }

    #[test]
    fn single_star_stays_inside_one_path_segment() {
        assert!(path_matches("/v3/*", "/v3/projects"));
        assert!(!path_matches("/v3/*", "/v3/projects/detail"));
    }

    #[test]
    fn double_star_crosses_segments() {
        assert!(path_matches("/v3/**", "/v3/projects/detail"));
        assert!(path_matches("/**", "/anything/at/all"));
    }

    #[test]
    fn patterns_are_anchored_at_both_ends() {
        assert!(!path_matches("/v3", "/v3/auth/tokens"));
        assert!(path_matches("/v3/auth/tokens", "/v3/auth/tokens"));
        assert!(!path_matches("/auth/tokens", "/v3/auth/tokens"));
    }
}
