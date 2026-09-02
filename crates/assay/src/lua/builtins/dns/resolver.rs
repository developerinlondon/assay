//! Who to ask, and how to get an answer out of them.
//!
//! UDP first, TCP when the answer does not fit — the ordinary resolver
//! bargain. The system's own nameservers are the default; there is no public
//! fallback, because a script that thinks it is asking the corporate resolver
//! should not silently ask someone else's.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use rand::RngExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

use super::wire::{self, Answer, Response};

pub const DEFAULT_PORT: u16 = 53;
const RESOLV_CONF: &str = "/etc/resolv.conf";

/// Comfortably over the 1232 bytes the query advertises, so a server that
/// ignores the advertised size still fits.
const RECV_BUFFER: usize = 4096;

/// The largest answer we will read over TCP. A DNS message cannot exceed
/// 65535 bytes by its own length prefix; the cap is here so a hostile server
/// cannot make us allocate on its say-so before the read.
const MAX_TCP_MESSAGE: usize = 65535;

pub struct Query {
    pub name: String,
    pub qtype: u16,
    pub servers: Vec<SocketAddr>,
    pub timeout: Duration,
    pub tries: u32,
}

/// Ask each server in turn, `tries` times round, and return the first answer.
///
/// The last failure is what surfaces, because "the resolver refused" and "the
/// resolver never replied" are different problems for whoever has to fix it.
pub async fn resolve(q: &Query) -> Result<Vec<Answer>, String> {
    if q.servers.is_empty() {
        return Err("no nameservers configured".to_string());
    }

    let mut last = String::new();
    for _ in 0..q.tries.max(1) {
        for server in &q.servers {
            match ask(server, q).await {
                Ok(answers) => return Ok(answers),
                Err(e) => last = format!("{server}: {e}"),
            }
        }
    }

    if q.tries > 1 {
        return Err(format!("{last} (after {} tries)", q.tries));
    }
    Err(last)
}

async fn ask(server: &SocketAddr, q: &Query) -> Result<Vec<Answer>, String> {
    let id = rand::rng().random::<u16>();
    let message = wire::encode_query(id, &q.name, q.qtype)?;

    let over_udp = exchange_udp(server, &message, id, q.qtype, q.timeout).await?;
    if !over_udp.truncated {
        return Ok(over_udp.answers);
    }
    // The answer did not fit in a datagram. Asking again over TCP is the
    // protocol's own remedy, not a retry.
    Ok(exchange_tcp(server, &message, id, q.qtype, q.timeout)
        .await?
        .answers)
}

async fn exchange_udp(
    server: &SocketAddr,
    message: &[u8],
    id: u16,
    qtype: u16,
    timeout: Duration,
) -> Result<Response, String> {
    let bind: SocketAddr = if server.is_ipv6() {
        "[::]:0".parse().expect("literal is a valid address")
    } else {
        "0.0.0.0:0".parse().expect("literal is a valid address")
    };

    let socket = UdpSocket::bind(bind)
        .await
        .map_err(|e| format!("bind: {e}"))?;
    socket
        .connect(*server)
        .await
        .map_err(|e| format!("connect: {e}"))?;
    socket
        .send(message)
        .await
        .map_err(|e| format!("send: {e}"))?;

    let deadline = tokio::time::Instant::now() + timeout;
    let mut buf = vec![0u8; RECV_BUFFER];
    loop {
        let read = tokio::time::timeout_at(deadline, socket.recv(&mut buf))
            .await
            .map_err(|_| format!("timeout after {}ms", timeout.as_millis()))?
            .map_err(|e| format!("recv: {e}"))?;

        // A reply carrying somebody else's transaction ID is not ours to
        // read; keep waiting for one that is, until the deadline says stop.
        if wire::message_id(&buf[..read]) != Some(id) {
            continue;
        }
        return wire::decode_response(&buf[..read], qtype);
    }
}

async fn exchange_tcp(
    server: &SocketAddr,
    message: &[u8],
    id: u16,
    qtype: u16,
    timeout: Duration,
) -> Result<Response, String> {
    let deadline = tokio::time::Instant::now() + timeout;
    let expired = || format!("timeout after {}ms", timeout.as_millis());

    let mut stream = tokio::time::timeout_at(deadline, TcpStream::connect(*server))
        .await
        .map_err(|_| expired())?
        .map_err(|e| format!("tcp connect: {e}"))?;

    // Over TCP a message is preceded by its own length.
    let mut framed = (message.len() as u16).to_be_bytes().to_vec();
    framed.extend_from_slice(message);
    tokio::time::timeout_at(deadline, stream.write_all(&framed))
        .await
        .map_err(|_| expired())?
        .map_err(|e| format!("tcp write: {e}"))?;

    let mut length = [0u8; 2];
    tokio::time::timeout_at(deadline, stream.read_exact(&mut length))
        .await
        .map_err(|_| expired())?
        .map_err(|e| format!("tcp read: {e}"))?;

    let len = u16::from_be_bytes(length) as usize;
    if len == 0 || len > MAX_TCP_MESSAGE {
        return Err(format!("tcp answer declares {len} bytes"));
    }
    let mut buf = vec![0u8; len];
    tokio::time::timeout_at(deadline, stream.read_exact(&mut buf))
        .await
        .map_err(|_| expired())?
        .map_err(|e| format!("tcp read: {e}"))?;

    if wire::message_id(&buf) != Some(id) {
        return Err("tcp answer carries the wrong transaction id".to_string());
    }
    wire::decode_response(&buf, qtype)
}

/// The nameservers the host is configured to use, in the order it lists them.
pub fn system_servers() -> Result<Vec<SocketAddr>, String> {
    let text = std::fs::read_to_string(RESOLV_CONF)
        .map_err(|e| format!("cannot read {RESOLV_CONF}: {e}"))?;
    let servers = parse_resolv_conf(&text);
    if servers.is_empty() {
        return Err(format!("{RESOLV_CONF} names no usable nameserver"));
    }
    Ok(servers)
}

/// Read the `nameserver` lines out of a `resolv.conf`, ignoring everything
/// else in it. `search`, `domain` and `options` shape name expansion, which
/// this builtin deliberately does not do — a caller asks for the name it means.
pub fn parse_resolv_conf(text: &str) -> Vec<SocketAddr> {
    let mut servers = Vec::new();
    for line in text.lines() {
        let body = match line.find(['#', ';']) {
            Some(at) => &line[..at],
            None => line,
        };
        let mut fields = body.split_whitespace();
        if fields.next() != Some("nameserver") {
            continue;
        }
        let Some(address) = fields.next() else {
            continue;
        };
        // A link-local v6 nameserver carries a `%eth0` scope that names an
        // interface rather than a host, and that `IpAddr` will not parse.
        let bare = address.split_once('%').map_or(address, |(ip, _)| ip);
        if let Ok(ip) = bare.parse::<IpAddr>() {
            servers.push(SocketAddr::new(ip, DEFAULT_PORT));
        }
    }
    servers
}

/// Parse an `opts.server`: a bare address, or one with an explicit port.
pub fn parse_server(spec: &str) -> Result<SocketAddr, String> {
    let spec = spec.trim();
    if let Ok(addr) = spec.parse::<SocketAddr>() {
        return Ok(addr);
    }
    if let Ok(ip) = spec.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, DEFAULT_PORT));
    }
    Err(format!(
        "'{spec}' is not a nameserver address — want an IP, 'IP:port', or '[v6]:port'"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nameservers_are_read_in_the_order_the_file_lists_them() {
        let conf = "\
# generated by something
search corp.example.com example.com
nameserver 10.0.0.53
options edns0 trust-ad
nameserver 10.0.1.53
domain corp.example.com
";
        assert_eq!(
            parse_resolv_conf(conf),
            vec![
                "10.0.0.53:53".parse::<SocketAddr>().unwrap(),
                "10.0.1.53:53".parse::<SocketAddr>().unwrap(),
            ]
        );
    }

    #[test]
    fn comments_and_unusable_entries_are_skipped() {
        let conf = "\
#nameserver 1.1.1.1
nameserver 8.8.8.8 # the one in use
; nameserver 9.9.9.9
nameserver not-an-address
nameserver
nameserver fe80::1%eth0
";
        assert_eq!(
            parse_resolv_conf(conf),
            vec![
                "8.8.8.8:53".parse::<SocketAddr>().unwrap(),
                "[fe80::1]:53".parse::<SocketAddr>().unwrap(),
            ]
        );
    }

    #[test]
    fn a_server_without_a_port_gets_the_standard_one() {
        assert_eq!(
            parse_server("1.1.1.1").unwrap(),
            "1.1.1.1:53".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(
            parse_server("1.1.1.1:5353").unwrap(),
            "1.1.1.1:5353".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(
            parse_server("[2606:4700:4700::1111]:5353").unwrap(),
            "[2606:4700:4700::1111]:5353".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(
            parse_server("2606:4700:4700::1111").unwrap(),
            "[2606:4700:4700::1111]:53".parse::<SocketAddr>().unwrap()
        );
        assert!(parse_server("resolver.example.com").is_err());
    }
}
