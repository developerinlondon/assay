//! The DNS message format: building a question, reading an answer.
//!
//! Pure byte handling, no I/O — which is what lets the awkward parts (chunked
//! TXT, compression pointers, the difference between "no such name" and "the
//! resolver broke") be tested without a nameserver.

use std::net::{Ipv4Addr, Ipv6Addr};

pub const TYPE_A: u16 = 1;
pub const TYPE_NS: u16 = 2;
pub const TYPE_CNAME: u16 = 5;
pub const TYPE_MX: u16 = 15;
pub const TYPE_TXT: u16 = 16;
pub const TYPE_AAAA: u16 = 28;
pub const TYPE_OPT: u16 = 41;

const CLASS_IN: u16 = 1;
const MAX_LABEL_LEN: usize = 63;
const MAX_NAME_LEN: usize = 255;

/// How large a UDP answer we tell the resolver we can accept. The classic
/// 512-byte limit truncates ordinary DKIM keys, so every lookup carries an
/// EDNS0 OPT record; 1232 is the size that survives the common path MTU.
const EDNS_UDP_PAYLOAD: u16 = 1232;

/// A compression pointer must move strictly backwards, so a chain terminates
/// on its own. The cap is belt-and-braces against a decoder change losing that
/// property.
const MAX_POINTER_JUMPS: usize = 32;

/// One record's worth of answer, in the shape Lua receives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// `A`, `AAAA`, `NS`, `CNAME` and `TXT` all answer with one string.
    Text(String),
    /// `MX` answers with two fields, and a caller that ignores preference
    /// picks the wrong mail host.
    Mx { preference: u16, exchange: String },
}

impl Answer {
    /// The address of an `A` answer, for callers that reason about the
    /// numbers rather than the text — the DNSBL rule being the one that does.
    pub fn as_ipv4(&self) -> Option<Ipv4Addr> {
        match self {
            Answer::Text(s) => s.parse().ok(),
            Answer::Mx { .. } => None,
        }
    }
}

#[derive(Debug)]
pub struct Response {
    /// The TC bit. A truncated answer is not a wrong answer, it is half an
    /// answer, and the caller is expected to ask again over TCP.
    pub truncated: bool,
    pub answers: Vec<Answer>,
}

/// The record types this builtin speaks, by their spelling in Lua.
pub fn record_type(name: &str) -> Option<u16> {
    match name.to_ascii_uppercase().as_str() {
        "A" => Some(TYPE_A),
        "AAAA" => Some(TYPE_AAAA),
        "NS" => Some(TYPE_NS),
        "CNAME" => Some(TYPE_CNAME),
        "MX" => Some(TYPE_MX),
        "TXT" => Some(TYPE_TXT),
        _ => None,
    }
}

pub const SUPPORTED_TYPES: &str = "A, AAAA, CNAME, MX, NS, TXT";

/// The transaction ID of a message, or `None` if it is too short to have one.
/// Read before decoding so a stray datagram can be dropped rather than parsed.
pub fn message_id(buf: &[u8]) -> Option<u16> {
    Some(u16::from_be_bytes([*buf.first()?, *buf.get(1)?]))
}

fn be16(buf: &[u8], at: usize) -> Result<u16, String> {
    let hi = *buf.get(at).ok_or("message ends mid-field")?;
    let lo = *buf.get(at + 1).ok_or("message ends mid-field")?;
    Ok(u16::from_be_bytes([hi, lo]))
}

/// Build a standard recursive query with one question and an EDNS0 OPT record.
pub fn encode_query(id: u16, name: &str, qtype: u16) -> Result<Vec<u8>, String> {
    let encoded = encode_name(name)?;

    let mut out = Vec::with_capacity(encoded.len() + 29);
    out.extend_from_slice(&id.to_be_bytes());
    out.extend_from_slice(&0x0100u16.to_be_bytes()); // RD
    out.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
    out.extend_from_slice(&0u16.to_be_bytes()); // ANCOUNT
    out.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT
    out.extend_from_slice(&1u16.to_be_bytes()); // ARCOUNT — the OPT below

    out.extend_from_slice(&encoded);
    out.extend_from_slice(&qtype.to_be_bytes());
    out.extend_from_slice(&CLASS_IN.to_be_bytes());

    // OPT pseudo-record: root name, type 41, "class" carries the UDP payload
    // size, and the four TTL bytes are extended-rcode/version/flags, all zero.
    out.push(0);
    out.extend_from_slice(&TYPE_OPT.to_be_bytes());
    out.extend_from_slice(&EDNS_UDP_PAYLOAD.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // RDLENGTH

    Ok(out)
}

fn encode_name(name: &str) -> Result<Vec<u8>, String> {
    let trimmed = name.trim_end_matches('.');
    if trimmed.is_empty() {
        return Ok(vec![0]);
    }

    let mut out = Vec::with_capacity(trimmed.len() + 2);
    for label in trimmed.split('.') {
        if label.is_empty() {
            return Err(format!("empty label in name '{name}'"));
        }
        if label.len() > MAX_LABEL_LEN {
            return Err(format!(
                "label '{label}' is {} bytes, over the {MAX_LABEL_LEN}-byte limit",
                label.len()
            ));
        }
        out.push(label.len() as u8);
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);

    if out.len() > MAX_NAME_LEN {
        return Err(format!(
            "name '{name}' encodes to {} bytes, over the {MAX_NAME_LEN}-byte limit",
            out.len()
        ));
    }
    Ok(out)
}

/// What a non-zero RCODE is called, so an error names the thing the resolver
/// actually said.
fn rcode_name(rcode: u16) -> String {
    match rcode {
        1 => "FORMERR".to_string(),
        2 => "SERVFAIL".to_string(),
        3 => "NXDOMAIN".to_string(),
        4 => "NOTIMP".to_string(),
        5 => "REFUSED".to_string(),
        other => format!("RCODE{other}"),
    }
}

/// Decode a response, keeping only answers of the type that was asked for.
///
/// NXDOMAIN is an empty list rather than an error: the name is absent, which
/// is a fact. Every other non-zero RCODE is an error, because the resolver has
/// told us it does not know — and a caller asking whether a domain is
/// blacklisted must not read "the resolver is down" as "listed on nothing".
pub fn decode_response(buf: &[u8], qtype: u16) -> Result<Response, String> {
    if buf.len() < 12 {
        return Err(format!("response is {} bytes, too short", buf.len()));
    }

    let flags = be16(buf, 2)?;
    if flags & 0x8000 == 0 {
        return Err("message is a query, not a response".to_string());
    }
    let truncated = flags & 0x0200 != 0;

    let rcode = flags & 0x000F;
    match rcode {
        0 => {}
        3 => {
            return Ok(Response {
                truncated: false,
                answers: Vec::new(),
            });
        }
        other => return Err(rcode_name(other)),
    }

    let qdcount = be16(buf, 4)?;
    let ancount = be16(buf, 6)?;

    let mut answers = Vec::new();
    let outcome = collect(buf, qdcount, ancount, qtype, &mut answers);
    // A truncated message is expected to end mid-record; the caller is about
    // to retry over TCP, so whatever parsed is a bonus rather than a verdict.
    if !truncated {
        outcome?;
    }

    if qtype == TYPE_MX {
        answers.sort_by_key(|a| match a {
            Answer::Mx { preference, .. } => *preference,
            Answer::Text(_) => u16::MAX,
        });
    }

    Ok(Response { truncated, answers })
}

fn collect(
    buf: &[u8],
    qdcount: u16,
    ancount: u16,
    qtype: u16,
    answers: &mut Vec<Answer>,
) -> Result<(), String> {
    let mut pos = 12;
    for _ in 0..qdcount {
        read_name(buf, &mut pos)?;
        pos += 4; // QTYPE + QCLASS
    }

    for _ in 0..ancount {
        read_name(buf, &mut pos)?;
        let rtype = be16(buf, pos)?;
        let class = be16(buf, pos + 2)?;
        let rdlength = be16(buf, pos + 8)? as usize;
        let rdata_at = pos + 10;
        if buf.len() < rdata_at + rdlength {
            return Err("record data runs past the end of the message".to_string());
        }

        // A response to an `A` query commonly carries the CNAME chain that led
        // to it. Filtering by type is what keeps those out of the answer.
        if rtype == qtype && class == CLASS_IN {
            answers.extend(parse_rdata(buf, qtype, rdata_at, rdlength)?);
        }
        pos = rdata_at + rdlength;
    }
    Ok(())
}

fn parse_rdata(
    buf: &[u8],
    qtype: u16,
    start: usize,
    len: usize,
) -> Result<Option<Answer>, String> {
    let rdata = &buf[start..start + len];
    match qtype {
        TYPE_A => {
            let octets: [u8; 4] = rdata
                .try_into()
                .map_err(|_| format!("A record has {len} bytes of data, want 4"))?;
            Ok(Some(Answer::Text(Ipv4Addr::from(octets).to_string())))
        }
        TYPE_AAAA => {
            let octets: [u8; 16] = rdata
                .try_into()
                .map_err(|_| format!("AAAA record has {len} bytes of data, want 16"))?;
            Ok(Some(Answer::Text(Ipv6Addr::from(octets).to_string())))
        }
        TYPE_NS | TYPE_CNAME => {
            let mut at = start;
            Ok(Some(Answer::Text(read_name(buf, &mut at)?)))
        }
        TYPE_MX => {
            let preference = be16(buf, start)?;
            let mut at = start + 2;
            let exchange = read_name(buf, &mut at)?;
            Ok(Some(Answer::Mx {
                preference,
                exchange,
            }))
        }
        TYPE_TXT => Ok(Some(Answer::Text(read_character_strings(rdata)?))),
        _ => Ok(None),
    }
}

/// One TXT record is a sequence of length-prefixed chunks, each at most 255
/// bytes. They are one value that the wire format had to cut up, so they are
/// joined with nothing between them — a separator would corrupt any DKIM key
/// long enough to need two chunks.
fn read_character_strings(rdata: &[u8]) -> Result<String, String> {
    let mut bytes = Vec::with_capacity(rdata.len());
    let mut at = 0;
    while at < rdata.len() {
        let len = rdata[at] as usize;
        at += 1;
        let end = at + len;
        if end > rdata.len() {
            return Err("TXT chunk runs past the end of the record".to_string());
        }
        bytes.extend_from_slice(&rdata[at..end]);
        at = end;
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Read a name, following compression pointers, and advance `pos` past the
/// name as it appeared here (not past wherever the pointers led).
///
/// Names come back lowercased and without the root dot, so two spellings of
/// the same host compare equal.
fn read_name(buf: &[u8], pos: &mut usize) -> Result<String, String> {
    let mut labels: Vec<String> = Vec::new();
    let mut cursor = *pos;
    let mut jumps = 0;
    let mut followed = false;
    let mut total = 1; // the root label

    loop {
        let len = *buf.get(cursor).ok_or("name runs past the end of the message")? as usize;
        cursor += 1;

        match len & 0xC0 {
            0x00 => {
                if len == 0 {
                    if !followed {
                        *pos = cursor;
                    }
                    return Ok(labels.join("."));
                }
                let end = cursor + len;
                let label = buf
                    .get(cursor..end)
                    .ok_or("label runs past the end of the message")?;
                total += 1 + len;
                if total > MAX_NAME_LEN {
                    return Err(format!("name exceeds {MAX_NAME_LEN} bytes"));
                }
                let text = std::str::from_utf8(label)
                    .map_err(|_| "label is not valid UTF-8".to_string())?;
                labels.push(text.to_ascii_lowercase());
                cursor = end;
            }
            0xC0 => {
                let lo = *buf
                    .get(cursor)
                    .ok_or("compression pointer runs past the end of the message")?;
                let pointer_at = cursor - 1;
                let target = ((len & 0x3F) << 8) | lo as usize;
                cursor += 1;
                if !followed {
                    *pos = cursor;
                    followed = true;
                }
                // Strictly backwards, so a pointer can neither name itself nor
                // start a cycle — a self-referential message errors instead of
                // hanging the resolver.
                if target >= pointer_at {
                    return Err("compression pointer does not point backwards".to_string());
                }
                jumps += 1;
                if jumps > MAX_POINTER_JUMPS {
                    return Err(format!("more than {MAX_POINTER_JUMPS} compression pointers"));
                }
                cursor = target;
            }
            _ => return Err("reserved label type in name".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Assemble a response whose answers all point their owner name at the
    /// question, which is what a real nameserver emits.
    fn response(flags: u16, question: (&str, u16), rrs: &[(u16, Vec<u8>)]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&0x1234u16.to_be_bytes());
        out.extend_from_slice(&flags.to_be_bytes());
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&(rrs.len() as u16).to_be_bytes());
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&0u16.to_be_bytes());

        out.extend_from_slice(&encode_name(question.0).unwrap());
        out.extend_from_slice(&question.1.to_be_bytes());
        out.extend_from_slice(&CLASS_IN.to_be_bytes());

        for (rtype, rdata) in rrs {
            out.extend_from_slice(&[0xC0, 0x0C]); // pointer to the question name
            out.extend_from_slice(&rtype.to_be_bytes());
            out.extend_from_slice(&CLASS_IN.to_be_bytes());
            out.extend_from_slice(&60u32.to_be_bytes());
            out.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
            out.extend_from_slice(rdata);
        }
        out
    }

    fn txt_rdata(chunks: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for chunk in chunks {
            out.push(chunk.len() as u8);
            out.extend_from_slice(chunk.as_bytes());
        }
        out
    }

    fn mx_rdata(preference: u16, exchange: &str) -> Vec<u8> {
        let mut out = preference.to_be_bytes().to_vec();
        out.extend_from_slice(&encode_name(exchange).unwrap());
        out
    }

    fn texts(answers: &[Answer]) -> Vec<String> {
        answers
            .iter()
            .map(|a| match a {
                Answer::Text(s) => s.clone(),
                other => panic!("expected text answer, got {other:?}"),
            })
            .collect()
    }

    #[test]
    fn txt_chunks_join_with_nothing_between_them() {
        let msg = response(
            0x8180,
            ("example.com", TYPE_TXT),
            &[(TYPE_TXT, txt_rdata(&["v=spf1 include:_spf.", "example.com ~all"]))],
        );
        let out = decode_response(&msg, TYPE_TXT).unwrap();
        assert_eq!(
            texts(&out.answers),
            vec!["v=spf1 include:_spf.example.com ~all"]
        );
    }

    #[test]
    fn a_dkim_key_split_across_two_chunks_reassembles_byte_exact() {
        // A 2048-bit key is longer than the 255 bytes one chunk can hold, so
        // every real one arrives in pieces.
        let head = "x".repeat(255);
        let tail = "y".repeat(120);
        let msg = response(
            0x8180,
            ("sel._domainkey.example.com", TYPE_TXT),
            &[(TYPE_TXT, txt_rdata(&[&head, &tail]))],
        );
        let out = decode_response(&msg, TYPE_TXT).unwrap();
        assert_eq!(texts(&out.answers), vec![format!("{head}{tail}")]);
    }

    #[test]
    fn two_txt_records_stay_two_strings() {
        let msg = response(
            0x8180,
            ("example.com", TYPE_TXT),
            &[
                (TYPE_TXT, txt_rdata(&["v=spf1 -all"])),
                (TYPE_TXT, txt_rdata(&["google-site-verification=abc"])),
            ],
        );
        let out = decode_response(&msg, TYPE_TXT).unwrap();
        assert_eq!(
            texts(&out.answers),
            vec!["v=spf1 -all", "google-site-verification=abc"]
        );
    }

    #[test]
    fn mx_carries_preference_and_comes_back_lowest_first() {
        let msg = response(
            0x8180,
            ("example.com", TYPE_MX),
            &[
                (TYPE_MX, mx_rdata(20, "ALT1.aspmx.L.google.com.")),
                (TYPE_MX, mx_rdata(1, "aspmx.l.google.com.")),
                (TYPE_MX, mx_rdata(10, "mx.example.com.")),
            ],
        );
        let out = decode_response(&msg, TYPE_MX).unwrap();
        assert_eq!(
            out.answers,
            vec![
                Answer::Mx {
                    preference: 1,
                    exchange: "aspmx.l.google.com".to_string()
                },
                Answer::Mx {
                    preference: 10,
                    exchange: "mx.example.com".to_string()
                },
                Answer::Mx {
                    preference: 20,
                    exchange: "alt1.aspmx.l.google.com".to_string()
                },
            ]
        );
    }

    #[test]
    fn nxdomain_is_an_empty_list_and_servfail_is_an_error() {
        let absent = response(0x8183, ("nope.example.com", TYPE_A), &[]);
        assert!(decode_response(&absent, TYPE_A).unwrap().answers.is_empty());

        let broken = response(0x8182, ("example.com", TYPE_A), &[]);
        assert_eq!(decode_response(&broken, TYPE_A).unwrap_err(), "SERVFAIL");

        let refused = response(0x8185, ("example.com", TYPE_A), &[]);
        assert_eq!(decode_response(&refused, TYPE_A).unwrap_err(), "REFUSED");

        let formerr = response(0x8181, ("example.com", TYPE_A), &[]);
        assert_eq!(decode_response(&formerr, TYPE_A).unwrap_err(), "FORMERR");
    }

    #[test]
    fn a_cname_alongside_the_answer_is_left_out_of_it() {
        let msg = response(
            0x8180,
            ("www.example.com", TYPE_A),
            &[
                (TYPE_CNAME, encode_name("example.com").unwrap()),
                (TYPE_A, vec![93, 184, 216, 34]),
            ],
        );
        let out = decode_response(&msg, TYPE_A).unwrap();
        assert_eq!(texts(&out.answers), vec!["93.184.216.34"]);
    }

    #[test]
    fn a_pointer_that_does_not_go_backwards_is_rejected_rather_than_followed() {
        // Header, then a name at offset 12 whose pointer names its own offset.
        let mut msg = vec![0; 12];
        msg[2] = 0x81;
        msg[3] = 0x80;
        msg[5] = 1; // QDCOUNT
        msg.extend_from_slice(&[0xC0, 0x0C]);
        msg.extend_from_slice(&TYPE_A.to_be_bytes());
        msg.extend_from_slice(&CLASS_IN.to_be_bytes());
        assert_eq!(
            decode_response(&msg, TYPE_A).unwrap_err(),
            "compression pointer does not point backwards"
        );

        // A pointer aimed past itself is the same fault in the other direction.
        let mut forward = msg.clone();
        forward[12] = 0xC0;
        forward[13] = 0x40;
        assert_eq!(
            decode_response(&forward, TYPE_A).unwrap_err(),
            "compression pointer does not point backwards"
        );
    }

    #[test]
    fn a_truncated_answer_reports_the_flag_instead_of_a_parse_error() {
        let mut msg = response(
            0x8180,
            ("example.com", TYPE_TXT),
            &[(TYPE_TXT, txt_rdata(&["v=spf1 -all"]))],
        );
        msg[2] |= 0x02; // TC
        msg.truncate(msg.len() - 6);
        let out = decode_response(&msg, TYPE_TXT).unwrap();
        assert!(out.truncated);
    }

    #[test]
    fn a_query_asks_recursively_and_advertises_an_edns_buffer() {
        let q = encode_query(0xBEEF, "example.com", TYPE_MX).unwrap();
        assert_eq!(message_id(&q), Some(0xBEEF));
        assert_eq!(&q[2..4], &0x0100u16.to_be_bytes()); // RD, no QR
        assert_eq!(&q[10..12], &1u16.to_be_bytes()); // one additional: the OPT
        assert_eq!(&q[12..25], b"\x07example\x03com\x00");
        assert!(q.ends_with(&[0, 0, 41, 0x04, 0xD0, 0, 0, 0, 0, 0, 0]));
    }

    #[test]
    fn names_that_cannot_be_encoded_are_refused_before_any_socket_is_opened() {
        assert!(encode_query(1, "exa..mple.com", TYPE_A).is_err());
        assert!(encode_query(1, &"a".repeat(64), TYPE_A).is_err());
        let too_long = vec!["abcdefghij"; 30].join(".");
        assert!(encode_query(1, &too_long, TYPE_A).is_err());
    }

    #[test]
    fn record_types_are_recognised_whatever_their_case() {
        assert_eq!(record_type("txt"), Some(TYPE_TXT));
        assert_eq!(record_type("Mx"), Some(TYPE_MX));
        assert_eq!(record_type("AAAA"), Some(TYPE_AAAA));
        assert_eq!(record_type("SOA"), None);
    }
}
