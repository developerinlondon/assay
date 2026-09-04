//! `dns.lookup` and `dns.dnsbl` driven through Lua against a stub nameserver.
//!
//! The stub binds loopback and answers with bytes this file assembles, so the
//! suite exercises the real socket path without a network — including the
//! answers a live resolver will not reliably produce on demand (SERVFAIL, a
//! silent server, a truncated reply, a blacklist's go-away code).

mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::run_lua;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};

const TYPE_A: u16 = 1;
const TYPE_CNAME: u16 = 5;
const TYPE_MX: u16 = 15;
const TYPE_TXT: u16 = 16;

/// A name in wire form. Queries never use compression, so plain labels are
/// all the stub ever needs to write.
fn name(n: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for label in n.trim_end_matches('.').split('.') {
        out.push(label.len() as u8);
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
    out
}

/// Where the question section ends: the labels, then QTYPE and QCLASS.
fn question_end(query: &[u8]) -> usize {
    let mut at = 12;
    while query[at] != 0 {
        at += 1 + query[at] as usize;
    }
    at + 1 + 4
}

/// The question as it was asked, so a stub can answer selectively.
fn question_name(query: &[u8]) -> String {
    let mut labels = Vec::new();
    let mut at = 12;
    while query[at] != 0 {
        let len = query[at] as usize;
        labels.push(String::from_utf8_lossy(&query[at + 1..at + 1 + len]).into_owned());
        at += 1 + len;
    }
    labels.join(".")
}

/// Echo the question back under the given RCODE, with one answer per record.
/// Owner names are the 0xC00C pointer a real server would emit.
fn answer(query: &[u8], rcode: u16, rrs: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&query[0..2]);
    out.extend_from_slice(&(0x8180u16 | rcode).to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&(rrs.len() as u16).to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&query[12..question_end(query)]);

    for (rtype, rdata) in rrs {
        out.extend_from_slice(&[0xC0, 0x0C]);
        out.extend_from_slice(&rtype.to_be_bytes());
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&60u32.to_be_bytes());
        out.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
        out.extend_from_slice(rdata);
    }
    out
}

/// An answer with the TC bit set and nothing in it — "ask again over TCP".
fn truncated(query: &[u8]) -> Vec<u8> {
    let mut out = answer(query, 0, &[]);
    out[2] |= 0x02;
    out
}

fn txt(chunks: &[&str]) -> Vec<u8> {
    let mut out = Vec::new();
    for chunk in chunks {
        out.push(chunk.len() as u8);
        out.extend_from_slice(chunk.as_bytes());
    }
    out
}

fn mx(preference: u16, exchange: &str) -> Vec<u8> {
    let mut out = preference.to_be_bytes().to_vec();
    out.extend_from_slice(&name(exchange));
    out
}

fn a(address: &str) -> Vec<u8> {
    address
        .split('.')
        .map(|o| o.parse::<u8>().unwrap())
        .collect()
}

/// Answer UDP queries on an already-bound socket. `reply` returning `None`
/// leaves the query unanswered, which is how the timeout and retry paths are
/// reached.
fn serve_udp<F>(socket: UdpSocket, reply: F)
where
    F: Fn(&[u8]) -> Option<Vec<u8>> + Send + 'static,
{
    tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        loop {
            let Ok((read, from)) = socket.recv_from(&mut buf).await else {
                return;
            };
            if let Some(out) = reply(&buf[..read]) {
                let _ = socket.send_to(&out, from).await;
            }
        }
    });
}

/// Answer TCP queries on an already-bound listener. Messages here carry their
/// own two-byte length.
fn serve_tcp<F>(listener: TcpListener, reply: F)
where
    F: Fn(&[u8]) -> Vec<u8> + Send + 'static,
{
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let mut framing = [0u8; 2];
            if sock.read_exact(&mut framing).await.is_err() {
                continue;
            }
            let mut query = vec![0u8; u16::from_be_bytes(framing) as usize];
            if sock.read_exact(&mut query).await.is_err() {
                continue;
            }
            let out = reply(&query);
            let mut framed = (out.len() as u16).to_be_bytes().to_vec();
            framed.extend_from_slice(&out);
            let _ = sock.write_all(&framed).await;
        }
    });
}

/// A nameserver on loopback, answering UDP only.
async fn nameserver<F>(reply: F) -> SocketAddr
where
    F: Fn(&[u8]) -> Option<Vec<u8>> + Send + 'static,
{
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = socket.local_addr().unwrap();
    serve_udp(socket, reply);
    addr
}

/// One loopback port, bound on both protocols.
///
/// UDP and TCP are independent port spaces, so a port the kernel hands out as
/// free on one says nothing about the other. The earlier version bound UDP,
/// then bound TCP on that same number and unwrapped it, which fails outright
/// the moment anything holds that TCP port.
///
/// The order carries no weight: binding UDP first and retrying the TCP half
/// would be exactly as correct. Two other things are what make this work. The
/// first socket stays bound while the second is attempted, so nothing can take
/// the candidate port in between and the pair is acquired or abandoned as a
/// unit. And a failed attempt is discarded whole, so each retry asks the kernel
/// for a fresh candidate rather than going back to a port already known to be
/// contended.
async fn bound_on_both() -> (UdpSocket, TcpListener, SocketAddr) {
    const ATTEMPTS: usize = 64;
    for _ in 0..ATTEMPTS {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        if let Ok(socket) = UdpSocket::bind(addr).await {
            return (socket, listener, addr);
        }
    }
    panic!("no loopback port free on both UDP and TCP in {ATTEMPTS} attempts");
}

/// A nameserver answering both protocols at one address, for the truncation
/// retry: the UDP half truncates and the TCP half carries the full answer.
async fn nameserver_both<U, T>(udp_reply: U, tcp_reply: T) -> SocketAddr
where
    U: Fn(&[u8]) -> Option<Vec<u8>> + Send + 'static,
    T: Fn(&[u8]) -> Vec<u8> + Send + 'static,
{
    let (socket, listener, addr) = bound_on_both().await;
    serve_udp(socket, udp_reply);
    serve_tcp(listener, tcp_reply);
    addr
}

#[tokio::test]
async fn txt_chunks_arrive_as_one_string() {
    let server = nameserver(|q| {
        Some(answer(
            q,
            0,
            &[
                (TYPE_TXT, txt(&["v=spf1 include:_spf.", "example.com ~all"])),
                (TYPE_TXT, txt(&["google-site-verification=xyz"])),
            ],
        ))
    })
    .await;

    let script = format!(
        r#"
        local txt = dns.lookup("example.com", "TXT", {{ server = "{server}" }})
        assert.eq(#txt, 2)
        assert.eq(txt[1], "v=spf1 include:_spf.example.com ~all")
        assert.eq(txt[2], "google-site-verification=xyz")
        "#
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn mx_answers_carry_preference_and_arrive_sorted() {
    let server = nameserver(|q| {
        Some(answer(
            q,
            0,
            &[
                (TYPE_MX, mx(20, "alt1.aspmx.l.google.com")),
                (TYPE_MX, mx(1, "aspmx.l.google.com")),
            ],
        ))
    })
    .await;

    let script = format!(
        r#"
        local mx = dns.lookup("example.com", "mx", {{ server = "{server}" }})
        assert.eq(#mx, 2)
        assert.eq(mx[1].preference, 1)
        assert.eq(mx[1].exchange, "aspmx.l.google.com")
        assert.eq(mx[2].preference, 20)
        assert.eq(mx[2].exchange, "alt1.aspmx.l.google.com")
        "#
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn a_lookup_skips_the_cname_that_led_to_it() {
    let server = nameserver(|q| {
        Some(answer(
            q,
            0,
            &[
                (TYPE_CNAME, name("example.com")),
                (TYPE_A, a("93.184.216.34")),
                (TYPE_A, a("93.184.216.35")),
            ],
        ))
    })
    .await;

    let script = format!(
        r#"
        local ips = dns.lookup("www.example.com", "A", {{ server = "{server}" }})
        assert.eq(#ips, 2)
        assert.eq(ips[1], "93.184.216.34")
        assert.eq(ips[2], "93.184.216.35")
        "#
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn nxdomain_is_an_empty_list_and_servfail_is_an_error() {
    let absent = nameserver(|q| Some(answer(q, 3, &[]))).await;
    let broken = nameserver(|q| Some(answer(q, 2, &[]))).await;

    let script = format!(
        r#"
        local none = dns.lookup("nope.example.com", "A", {{ server = "{absent}" }})
        assert.eq(#none, 0)

        local ok, err = pcall(function()
            dns.lookup("example.com", "A", {{ server = "{broken}" }})
        end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "SERVFAIL")
        assert.contains(tostring(err), "dns.lookup example.com A")
        "#
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn a_silent_resolver_times_out_rather_than_answering_empty() {
    let server = nameserver(|_| None).await;

    let script = format!(
        r#"
        local ok, err = pcall(function()
            dns.lookup("example.com", "A",
                {{ server = "{server}", timeout_ms = 200, tries = 1 }})
        end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "timeout after 200ms")
        "#
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn a_dropped_query_is_asked_again() {
    let seen = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&seen);
    let server = nameserver(move |q| {
        // Ignore the first query outright, the way a loaded resolver does.
        if counter.fetch_add(1, Ordering::SeqCst) == 0 {
            return None;
        }
        Some(answer(q, 0, &[(TYPE_A, a("198.51.100.7"))]))
    })
    .await;

    let script = format!(
        r#"
        local ips = dns.lookup("example.com", "A",
            {{ server = "{server}", timeout_ms = 200, tries = 2 }})
        assert.eq(#ips, 1)
        assert.eq(ips[1], "198.51.100.7")
        "#
    );
    run_lua(&script).await.unwrap();
    assert_eq!(seen.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_truncated_answer_is_asked_again_over_tcp() {
    let key = "k".repeat(300);
    let over_tcp = key.clone();
    let server = nameserver_both(
        |q| Some(truncated(q)),
        move |q| {
            let (head, tail) = over_tcp.split_at(255);
            answer(q, 0, &[(TYPE_TXT, txt(&[head, tail]))])
        },
    )
    .await;

    let script = format!(
        r#"
        local txt = dns.lookup("sel._domainkey.example.com", "TXT",
            {{ server = "{server}" }})
        assert.eq(#txt, 1)
        assert.eq(txt[1], "{key}")
        "#
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn an_unsupported_record_type_is_named_rather_than_answered_empty() {
    let script = r#"
        local ok, err = pcall(function() dns.lookup("example.com", "SOA") end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "unsupported record type 'SOA'")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn dnsbl_asks_about_the_domain_under_the_list() {
    let server = nameserver(|q| {
        // Only the composed name is a hit; anything else is absent. A wrongly
        // built query therefore reads as "not listed" and fails the assertion.
        if question_name(q) == "bad.example.com.fresh.spameatingmonkey.net" {
            return Some(answer(q, 0, &[(TYPE_A, a("127.0.0.2"))]));
        }
        Some(answer(q, 3, &[]))
    })
    .await;

    let script = format!(
        r#"
        local hit = dns.dnsbl("bad.example.com", "fresh.spameatingmonkey.net",
            {{ server = "{server}" }})
        assert.eq(hit.listed, true)
        assert.eq(#hit.codes, 1)
        assert.eq(hit.codes[1], "127.0.0.2")

        local clean = dns.dnsbl("good.example.com", "fresh.spameatingmonkey.net",
            {{ server = "{server}" }})
        assert.eq(clean.listed, false)
        assert.eq(#clean.codes, 0)
        "#
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn the_go_away_code_is_reported_without_being_called_a_listing() {
    let server = nameserver(|q| Some(answer(q, 0, &[(TYPE_A, a("127.255.255.254"))]))).await;

    let script = format!(
        r#"
        local r = dns.dnsbl("example.com", "zen.spamhaus.org", {{ server = "{server}" }})
        assert.eq(r.listed, false)
        -- Still reported, so a caller can tell "not listed" from "not allowed to ask".
        assert.eq(#r.codes, 1)
        assert.eq(r.codes[1], "127.255.255.254")
        "#
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn a_broken_resolver_fails_a_dnsbl_check_rather_than_clearing_it() {
    let server = nameserver(|q| Some(answer(q, 2, &[]))).await;

    let script = format!(
        r#"
        local ok, err = pcall(function()
            dns.dnsbl("example.com", "zen.spamhaus.org", {{ server = "{server}" }})
        end)
        assert.eq(ok, false)
        assert.contains(tostring(err), "SERVFAIL")
        "#
    );
    run_lua(&script).await.unwrap();
}
