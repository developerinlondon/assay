//! The SMTP wire format and what a rejection means.

use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

const MAX_REPLY_LINES: usize = 64;
const MAX_DETAIL_LEN: usize = 300;

pub type Session = BufReader<TcpStream>;

pub struct Reply {
    pub code: u16,
    pub text: String,
}

impl Reply {
    pub fn positive(&self) -> bool {
        (200..400).contains(&self.code)
    }

    pub fn reason(&self) -> &'static str {
        if self.positive() {
            "accepted"
        } else {
            classify(self.code, &self.text)
        }
    }
}

/// What a rejection means, in the vocabulary `email_verify` maps to verdicts.
///
/// Several of these codes carry a meaning no RFC mandates, which is why the
/// reply text is consulted before the code wherever operators disagree.
pub fn classify(code: u16, text: &str) -> &'static str {
    let lowered = text.to_lowercase();
    let says = |needles: &[&str]| needles.iter().any(|n| lowered.contains(n));

    // A server that names the mailbox as the problem has told us the answer
    // regardless of which 4xx or 5xx it chose to wrap it in.
    if says(MAILBOX_ABSENT) {
        return "no_mailbox";
    }
    if says(REPUTATION) {
        return "blocked";
    }

    match code {
        421 => "try_again_later",
        450 => "mailbox_busy",
        451 => "exceeded_limits",
        452 if says(OUT_OF_SPACE) => "full_inbox",
        452 => "too_many_recipients",
        503 => "need_mail_before_rcpt",
        550 => "no_mailbox",
        551 => "recipient_moved",
        552 => "full_inbox",
        553 => "no_relay",
        554 => "not_allowed",
        _ => "rejected",
    }
}

const MAILBOX_ABSENT: &[&str] = &[
    "undeliverable",
    "does not exist",
    "doesn't exist",
    "may not exist",
    "user unknown",
    "unknown user",
    "user not found",
    "no such user",
    "no such recipient",
    "invalid address",
    "invalid recipient",
    "recipient invalid",
    "recipient rejected",
    "address rejected",
    "no mailbox",
    "mailbox unavailable",
    "mailbox not found",
];

const REPUTATION: &[&str] = &[
    "spamhaus",
    "proofpoint",
    "cloudmark",
    "barracuda",
    "blacklist",
    "block list",
    "blocklist",
    "blocked",
    "banned",
    "denied",
    "reputation",
];

const OUT_OF_SPACE: &[&str] = &[
    "full",
    "space",
    "over quota",
    "insufficient",
    "exceeded storage",
];

/// Codes a server uses to say "not now" rather than "not ever". A probe that
/// stops here has learned nothing about the mailbox, which is a different
/// answer from learning it is absent.
pub fn is_greylist(reason: &str) -> bool {
    matches!(
        reason,
        "try_again_later" | "mailbox_busy" | "exceeded_limits"
    )
}

pub fn truncate(s: &str) -> String {
    let cleaned = s.replace(['\r', '\n'], " ");
    let trimmed = cleaned.trim();
    if trimmed.chars().count() <= MAX_DETAIL_LEN {
        return trimmed.to_string();
    }
    trimmed.chars().take(MAX_DETAIL_LEN).collect()
}

/// Read one complete reply, following the `250-continued` / `250 final`
/// continuation rule. A stream that ends mid-reply is an error, not an empty
/// success.
pub async fn read_reply(session: &mut Session, op: Duration) -> Result<Reply, String> {
    let mut parts: Vec<String> = Vec::new();
    for _ in 0..MAX_REPLY_LINES {
        let mut line = String::new();
        let read = tokio::time::timeout(op, session.read_line(&mut line))
            .await
            .map_err(|_| "timeout".to_string())?
            .map_err(|e| e.to_string())?;
        if read == 0 {
            return Err("connection closed".to_string());
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.len() < 3 {
            return Err(format!("malformed reply: {line}"));
        }
        let code = line[..3]
            .parse::<u16>()
            .map_err(|_| format!("malformed reply: {line}"))?;
        parts.push(line[3..].trim_start_matches(['-', ' ']).to_string());
        // A hyphen in the fourth column continues the reply; anything else ends it.
        if line.as_bytes().get(3).is_none_or(|b| *b != b'-') {
            return Ok(Reply {
                code,
                text: parts.join(" "),
            });
        }
    }
    Err(format!("reply exceeded {MAX_REPLY_LINES} lines"))
}

pub async fn send(session: &mut Session, line: &str, op: Duration) -> Result<Reply, String> {
    let payload = format!("{line}\r\n");
    tokio::time::timeout(op, session.get_mut().write_all(payload.as_bytes()))
        .await
        .map_err(|_| "timeout".to_string())?
        .map_err(|e| e.to_string())?;
    read_reply(session, op).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailbox_wording_outranks_the_code_it_arrives_with() {
        assert_eq!(classify(552, "5.1.1 user unknown"), "no_mailbox");
        assert_eq!(classify(452, "4.2.2 mailbox is full"), "full_inbox");
        assert_eq!(classify(452, "too many recipients"), "too_many_recipients");
        assert_eq!(classify(550, "5.7.1 blocked by spamhaus"), "blocked");
        assert_eq!(classify(550, "5.1.1 unrouteable"), "no_mailbox");
        assert_eq!(classify(421, "service closing"), "try_again_later");
        assert_eq!(classify(599, "who knows"), "rejected");
    }

    #[test]
    fn only_soft_refusals_count_as_greylisting() {
        assert!(is_greylist("try_again_later"));
        assert!(is_greylist("mailbox_busy"));
        assert!(is_greylist("exceeded_limits"));
        assert!(!is_greylist("no_mailbox"));
        assert!(!is_greylist("accepted"));
    }

    #[test]
    fn detail_is_flattened_and_bounded() {
        assert_eq!(truncate("  550 no\r\n such user \n"), "550 no   such user");
        assert_eq!(truncate(&"x".repeat(400)).chars().count(), MAX_DETAIL_LEN);
    }
}
