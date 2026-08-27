//! The envelope dialogue: connect, greeting, EHLO, MAIL FROM, RCPT TO, QUIT.
//!
//! DATA is never issued, so a probe can never deliver anything. Name
//! resolution and verdict vocabulary stay in `assay.email_verify`.

use super::reply::{Reply, Session, is_greylist, read_reply, send, truncate};
use rand::RngExt;
use std::time::Duration;
use tokio::io::BufReader;
use tokio::net::TcpStream;

pub struct Params {
    pub email: String,
    pub domain: String,
    pub hosts: Vec<String>,
    pub from: String,
    pub helo: String,
    pub port: u16,
    pub connect: Duration,
    pub op: Duration,
    pub catch_all_check: bool,
    pub greylist_delay: Duration,
}

#[derive(Default)]
pub struct Probe {
    pub host_exists: bool,
    pub mx_host: Option<String>,
    pub catch_all: bool,
    pub deliverable: bool,
    pub full_inbox: bool,
    pub disabled: bool,
    pub blocked: bool,
    pub greylisted: bool,
    pub code: Option<u16>,
    pub reason: String,
    pub detail: String,
    pub stage: String,
}

impl Probe {
    fn stopped(mut self, stage: &str, reason: &str, detail: String) -> Self {
        self.stage = stage.to_string();
        self.reason = reason.to_string();
        self.detail = detail;
        self
    }

    fn note_flags(&mut self, reason: &str) {
        self.full_inbox |= reason == "full_inbox";
        self.disabled |= reason == "not_allowed";
        self.blocked |= reason == "blocked";
    }

    fn rejected_at(mut self, stage: &str, reply: &Reply) -> Self {
        self.code = Some(reply.code);
        let reason = reply.reason();
        self.note_flags(reason);
        self.stopped(stage, reason, truncate(&reply.text))
    }
}

/// Connect to the first MX host that answers with a greeting.
///
/// Tried in the given order rather than raced: the caller has already sorted
/// them by preference, and one connection at a time is the neighbourly shape
/// for something pointed at other people's servers.
async fn connect_any(p: &Params) -> Result<(Session, String), String> {
    let mut last = "no mx hosts".to_string();
    for host in &p.hosts {
        let addr = format!("{host}:{}", p.port);
        let stream = match tokio::time::timeout(p.connect, TcpStream::connect(&addr)).await {
            Err(_) => {
                last = format!("{host}: connect timeout");
                continue;
            }
            Ok(Err(e)) => {
                last = format!("{host}: {e}");
                continue;
            }
            Ok(Ok(s)) => s,
        };
        let mut session = BufReader::new(stream);
        match read_reply(&mut session, p.op).await {
            Ok(greeting) if greeting.code == 220 => return Ok((session, host.clone())),
            Ok(other) => last = format!("{host}: greeting {} {}", other.code, other.text),
            Err(e) => last = format!("{host}: {e}"),
        }
    }
    Err(last)
}

/// ESMTP is not universal; a server that refuses EHLO may still speak HELO.
async fn say_hello(session: &mut Session, p: &Params) -> Result<Reply, String> {
    let ehlo = send(session, &format!("EHLO {}", p.helo), p.op).await?;
    if ehlo.positive() {
        return Ok(ehlo);
    }
    send(session, &format!("HELO {}", p.helo), p.op).await
}

/// Offer an address nobody owns. Acceptance means this host says yes to
/// everything, so the target address could prove nothing here.
async fn catch_all_probe(session: &mut Session, p: &Params, out: &mut Probe) -> Option<Probe> {
    let random = format!("{:016x}@{}", rand::rng().random::<u64>(), p.domain);
    let reply = match send(session, &format!("RCPT TO:<{random}>"), p.op).await {
        Ok(r) => r,
        Err(e) => {
            let stopped = std::mem::take(out);
            return Some(stopped.stopped("rcpt_random", "io_error", truncate(&e)));
        }
    };
    out.code = Some(reply.code);
    if reply.positive() {
        out.catch_all = true;
        let _ = send(session, "QUIT", p.op).await;
        let stopped = std::mem::take(out);
        return Some(stopped.stopped("rcpt_random", "catch_all", truncate(&reply.text)));
    }
    out.note_flags(reply.reason());
    out.detail = truncate(&reply.text);
    None
}

/// Greylisting is a deliberate soft refusal of a first attempt. One delayed
/// retry converts the common case into an answer; a server still refusing has
/// told us to come back later, which is not the same as telling us the mailbox
/// is absent.
async fn rcpt_target(session: &mut Session, p: &Params) -> Result<Reply, String> {
    let command = format!("RCPT TO:<{}>", p.email);
    let first = send(session, &command, p.op).await?;
    if !is_greylist(first.reason()) || p.greylist_delay.is_zero() {
        return Ok(first);
    }
    tokio::time::sleep(p.greylist_delay).await;
    Ok(send(session, &command, p.op).await.unwrap_or(first))
}

pub async fn run(p: Params) -> Probe {
    let (mut session, host) = match connect_any(&p).await {
        Ok(v) => v,
        Err(detail) => {
            return Probe::default().stopped("connect", "unreachable", truncate(&detail));
        }
    };

    // The greeting was read, so the host is there no matter what it says next.
    // Recording that before EHLO keeps a rejection of *us* from being reported
    // as an absent host.
    let mut out = Probe {
        host_exists: true,
        mx_host: Some(host),
        ..Default::default()
    };

    match say_hello(&mut session, &p).await {
        Err(e) => return out.stopped("ehlo", "io_error", truncate(&e)),
        Ok(r) if !r.positive() => return out.rejected_at("ehlo", &r),
        Ok(_) => {}
    }

    match send(&mut session, &format!("MAIL FROM:<{}>", p.from), p.op).await {
        Err(e) => return out.stopped("mail_from", "io_error", truncate(&e)),
        Ok(r) if !r.positive() => return out.rejected_at("mail_from", &r),
        Ok(_) => {}
    }

    if p.catch_all_check
        && let Some(finished) = catch_all_probe(&mut session, &p, &mut out).await
    {
        return finished;
    }

    let reply = match rcpt_target(&mut session, &p).await {
        Ok(r) => r,
        Err(e) => return out.stopped("rcpt_target", "io_error", truncate(&e)),
    };
    let _ = send(&mut session, "QUIT", p.op).await;

    let reason = reply.reason();
    out.code = Some(reply.code);
    out.deliverable = reply.positive();
    out.greylisted = is_greylist(reason);
    out.note_flags(reason);
    out.stopped("rcpt_target", reason, truncate(&reply.text))
}
