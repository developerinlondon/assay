//! `dns` — name lookups, and the blacklist question they are usually asked for.
//!
//! Domain health is the driving case: MX to see whether mail can arrive, TXT
//! to read SPF, DKIM and DMARC, and a DNSBL check to see who has taken against
//! the domain. All of that needs record types `getaddrinfo` cannot ask for,
//! which is why this speaks the protocol rather than calling the stub resolver.

mod resolver;
mod wire;

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use mlua::{Lua, Table};

use resolver::Query;
use wire::Answer;

const DEFAULT_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_TRIES: u32 = 2;

/// What Spamhaus, SURBL and their imitators answer a public resolver with
/// instead of a verdict: the query was answered, but the answer is "you are
/// not entitled to ask". Reading it as a listing marks every domain checked as
/// blacklisted, which is the one mistake a blacklist check cannot afford.
const PUBLIC_RESOLVER_MARKER: [u8; 3] = [127, 255, 255];

fn opt_string(opts: Option<&Table>, key: &str) -> mlua::Result<Option<String>> {
    match opts {
        Some(t) => t.get::<Option<String>>(key),
        None => Ok(None),
    }
}

fn opt_number<T: mlua::FromLua>(opts: Option<&Table>, key: &str) -> mlua::Result<Option<T>> {
    match opts {
        Some(t) => t.get::<Option<T>>(key),
        None => Ok(None),
    }
}

/// Where to send the question.
///
/// A caller-chosen nameserver is a convenience under no policy and an egress
/// channel under one: a restricted script could otherwise carry data out in
/// the names it looks up, to a server it also chooses. So with a policy
/// installed the option is refused rather than quietly swapped for the system
/// resolver, which would answer a different question than the one that was
/// asked without saying so.
fn servers_for(lua: &Lua, fname: &str, opts: Option<&Table>) -> mlua::Result<Vec<SocketAddr>> {
    let Some(spec) = opt_string(opts, "server")? else {
        return resolver::system_servers()
            .map_err(|e| mlua::Error::runtime(format!("{fname}: {e}")));
    };
    if crate::lua::policy::active(lua).is_some() {
        return Err(mlua::Error::runtime(format!(
            "{fname}: opts.server is not allowed while a policy is installed"
        )));
    }
    let server =
        resolver::parse_server(&spec).map_err(|e| mlua::Error::runtime(format!("{fname}: {e}")))?;
    Ok(vec![server])
}

fn query_for(
    lua: &Lua,
    fname: &str,
    name: &str,
    qtype: u16,
    opts: Option<&Table>,
) -> mlua::Result<Query> {
    let timeout_ms = opt_number::<u64>(opts, "timeout_ms")?.unwrap_or(DEFAULT_TIMEOUT_MS);
    let tries = opt_number::<u32>(opts, "tries")?.unwrap_or(DEFAULT_TRIES);
    Ok(Query {
        name: name.to_string(),
        qtype,
        servers: servers_for(lua, fname, opts)?,
        timeout: Duration::from_millis(timeout_ms),
        tries: tries.max(1),
    })
}

fn answers_to_table(lua: &Lua, answers: Vec<Answer>) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    for (i, answer) in answers.into_iter().enumerate() {
        match answer {
            Answer::Text(s) => out.set(i + 1, s)?,
            Answer::Mx {
                preference,
                exchange,
            } => {
                let record = lua.create_table()?;
                record.set("preference", preference)?;
                record.set("exchange", exchange)?;
                out.set(i + 1, record)?;
            }
        }
    }
    Ok(out)
}

/// Whether a DNSBL's reply means "listed".
///
/// A list answers in `127.0.0.0/8`, using the low octets to say which of its
/// sub-lists matched. Everything in that range counts except the whole of
/// `127.255.255.0/24`, which the big lists reserve for turning away resolvers
/// they do not serve.
fn listed(codes: &[Ipv4Addr]) -> bool {
    codes.iter().any(|ip| {
        let [a, b, c, _] = ip.octets();
        a == 127 && [a, b, c] != PUBLIC_RESOLVER_MARKER
    })
}

pub fn register_dns(lua: &Lua) -> mlua::Result<()> {
    let table = lua.create_table()?;

    let lookup = lua.create_async_function(
        |lua, (name, rtype, opts): (String, String, Option<Table>)| async move {
            let qtype = wire::record_type(&rtype).ok_or_else(|| {
                mlua::Error::runtime(format!(
                    "dns.lookup: unsupported record type '{rtype}' — want one of {}",
                    wire::SUPPORTED_TYPES
                ))
            })?;
            let query = query_for(&lua, "dns.lookup", &name, qtype, opts.as_ref())?;
            let answers = resolver::resolve(&query).await.map_err(|e| {
                let rtype = rtype.to_ascii_uppercase();
                mlua::Error::runtime(format!("dns.lookup {name} {rtype}: {e}"))
            })?;
            answers_to_table(&lua, answers)
        },
    )?;
    table.set("lookup", lookup)?;

    let dnsbl = lua.create_async_function(
        |lua, (domain, list, opts): (String, String, Option<Table>)| async move {
            let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
            let list = list.trim().trim_end_matches('.').to_ascii_lowercase();
            if domain.is_empty() || list.is_empty() {
                return Err(mlua::Error::runtime(
                    "dns.dnsbl: both a domain and a list are required",
                ));
            }

            let probe = format!("{domain}.{list}");
            let query = query_for(&lua, "dns.dnsbl", &probe, wire::TYPE_A, opts.as_ref())?;
            // A resolver failure is reported, never folded into `listed =
            // false`. "Nobody lists it" and "nobody answered" look identical
            // from here and mean opposite things to whoever is sending mail.
            let answers = resolver::resolve(&query)
                .await
                .map_err(|e| mlua::Error::runtime(format!("dns.dnsbl {domain} {list}: {e}")))?;

            let addresses: Vec<Ipv4Addr> = answers.iter().filter_map(Answer::as_ipv4).collect();
            let out = lua.create_table()?;
            out.set("listed", listed(&addresses))?;
            // Every code is reported whatever the verdict, so a caller can see
            // which sub-list matched — or that the answer was the go-away code.
            out.set("codes", answers_to_table(&lua, answers)?)?;
            Ok(out)
        },
    )?;
    table.set("dnsbl", dnsbl)?;

    lua.globals().set("dns", table)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ips(addresses: &[&str]) -> Vec<Ipv4Addr> {
        addresses.iter().map(|a| a.parse().unwrap()).collect()
    }

    #[test]
    fn an_ordinary_loopback_code_is_a_listing() {
        assert!(listed(&ips(&["127.0.0.2"])));
        assert!(listed(&ips(&["127.0.0.10"])));
        assert!(listed(&ips(&["127.0.1.2"])));
    }

    #[test]
    fn the_go_away_code_public_resolvers_get_is_not_a_listing() {
        assert!(!listed(&ips(&["127.255.255.254"])));
        assert!(!listed(&ips(&["127.255.255.0"])));
        assert!(!listed(&ips(&["127.255.255.255"])));
    }

    #[test]
    fn the_excluded_range_stops_at_its_own_edge() {
        // One address below `127.255.255.0` is an ordinary listing again.
        assert!(listed(&ips(&["127.255.254.255"])));
    }

    #[test]
    fn nothing_and_nothing_loopback_are_not_listings() {
        assert!(!listed(&[]));
        assert!(!listed(&ips(&["10.0.0.1"])));
        assert!(!listed(&ips(&["0.0.0.0"])));
    }

    #[test]
    fn a_real_hit_alongside_the_go_away_code_is_still_a_hit() {
        assert!(listed(&ips(&["127.255.255.254", "127.0.0.4"])));
    }
}
