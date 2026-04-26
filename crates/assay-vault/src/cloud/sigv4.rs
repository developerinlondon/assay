//! AWS Signature Version 4 — minimal implementation.
//!
//! Implements just the canonical-request + string-to-sign + signing-key
//! flow from the AWS docs ("Examples of the complete Version 4 signing
//! process"). Header signing only (not query-string presigning); that
//! covers KMS, STS, IAM, S3 — every cloud surface the vault needs.
//!
//! Plan 17 §"Cost estimate" budgets ~150 KB for the AWS surface. The
//! full `aws-sigv4` crate is ~1 MB plus its smithy dep tree; this
//! module is ~250 LOC and the only crypto primitives it pulls are
//! `sha2` + `hmac`, both already in the dep tree.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

type HmacSha256 = Hmac<Sha256>;

/// One signed request — all the fields the caller needs to construct an
/// `http::Request` against the target service.
#[derive(Clone, Debug)]
pub struct SignedRequest {
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Inputs to [`sign`].
#[derive(Clone, Copy, Debug)]
pub struct SigV4Input<'a> {
    /// IAM access key ID.
    pub access_key_id: &'a str,
    /// IAM secret access key.
    pub secret_access_key: &'a str,
    /// Optional STS session token (for assumed roles).
    pub session_token: Option<&'a str>,
    pub region: &'a str,
    pub service: &'a str,
    pub method: &'a str,
    /// e.g. "https://kms.us-east-1.amazonaws.com/"
    pub url: &'a str,
    /// Caller-supplied headers; `host` and `x-amz-date` are added
    /// automatically and don't need to be in this list.
    pub headers: &'a [(&'a str, &'a str)],
    pub body: &'a [u8],
    /// UTC timestamp formatted as `YYYYMMDDTHHMMSSZ`. Caller passes
    /// it explicitly so tests can pin it; production callers use
    /// [`now_amz_date`].
    pub amz_date: &'a str,
}

/// Build a `YYYYMMDDTHHMMSSZ` AMZ-date string for the current time.
pub fn now_amz_date() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format_amz_date(now)
}

fn format_amz_date(epoch_secs: u64) -> String {
    let dt: chrono::DateTime<chrono::Utc> =
        chrono::DateTime::from_timestamp(epoch_secs as i64, 0).unwrap_or_default();
    dt.format("%Y%m%dT%H%M%SZ").to_string()
}

/// Sign a request and return the URL + final headers + body.
pub fn sign(input: SigV4Input<'_>) -> SignedRequest {
    let date_only = &input.amz_date[..8]; // YYYYMMDD
    let credential_scope =
        format!("{date_only}/{}/{}/aws4_request", input.region, input.service);

    let parsed = url::Url::parse(input.url).expect("sigv4 input.url must parse");
    let host = parsed
        .host_str()
        .expect("sigv4 input.url must have a host")
        .to_string();
    let canonical_uri = if parsed.path().is_empty() {
        "/".to_string()
    } else {
        parsed.path().to_string()
    };
    let canonical_query = canonicalize_query(&parsed);

    // Canonical headers — sorted lowercase. Always include host +
    // x-amz-date + x-amz-content-sha256; merge any caller-supplied
    // headers in.
    let body_sha = hex(&Sha256::digest(input.body));
    let mut header_map: BTreeMap<String, String> = BTreeMap::new();
    header_map.insert("host".into(), host.clone());
    header_map.insert("x-amz-date".into(), input.amz_date.into());
    header_map.insert("x-amz-content-sha256".into(), body_sha.clone());
    if let Some(t) = input.session_token {
        header_map.insert("x-amz-security-token".into(), t.into());
    }
    for (k, v) in input.headers {
        header_map.insert(k.to_ascii_lowercase(), (*v).trim().to_string());
    }
    let mut canonical_headers = String::new();
    let mut signed_headers_parts: Vec<&str> = Vec::new();
    for (k, v) in &header_map {
        canonical_headers.push_str(k);
        canonical_headers.push(':');
        canonical_headers.push_str(v);
        canonical_headers.push('\n');
        signed_headers_parts.push(k);
    }
    let signed_headers = signed_headers_parts.join(";");

    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        input.method.to_ascii_uppercase(),
        canonical_uri,
        canonical_query,
        canonical_headers,
        signed_headers,
        body_sha,
    );
    let canonical_request_hash = hex(&Sha256::digest(canonical_request.as_bytes()));

    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{}\n{}",
        input.amz_date, credential_scope, canonical_request_hash
    );

    // Derive signing key: kSecret → kDate → kRegion → kService → kSigning.
    let k_date = hmac(
        format!("AWS4{}", input.secret_access_key).as_bytes(),
        date_only.as_bytes(),
    );
    let k_region = hmac(&k_date, input.region.as_bytes());
    let k_service = hmac(&k_region, input.service.as_bytes());
    let k_signing = hmac(&k_service, b"aws4_request");
    let signature = hex(&hmac(&k_signing, string_to_sign.as_bytes()));

    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
        input.access_key_id, credential_scope, signed_headers, signature
    );

    let mut out_headers: Vec<(String, String)> = header_map
        .into_iter()
        .map(|(k, v)| (k, v))
        .collect();
    out_headers.push(("authorization".into(), authorization));

    SignedRequest {
        url: input.url.to_string(),
        method: input.method.to_string(),
        headers: out_headers,
        body: input.body.to_vec(),
    }
}

fn canonicalize_query(url: &url::Url) -> String {
    let mut pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (uri_encode(&k, true), uri_encode(&v, true)))
        .collect();
    pairs.sort();
    pairs
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

/// Per AWS sigv4 spec — encode every byte except A-Za-z0-9-._~.
/// `encode_slash = true` for query strings; for paths AWS leaves `/`
/// unencoded (handled in canonical_uri above).
fn uri_encode(input: &str, encode_slash: bool) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.as_bytes() {
        let c = *b as char;
        let unreserved = matches!(c, 'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~');
        if unreserved || (!encode_slash && c == '/') {
            out.push(c);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

fn hmac(key: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut m = HmacSha256::new_from_slice(key).expect("hmac key");
    m.update(msg);
    m.finalize().into_bytes().to_vec()
}

fn hex(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic self-test — same inputs produce same signature
    /// twice, and the credential scope is well-formed. End-to-end
    /// validation against real AWS happens against localstack per
    /// plan §"Test plan".
    #[test]
    fn signature_is_deterministic() {
        let inputs = SigV4Input {
            access_key_id: "AKIDEXAMPLE",
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            session_token: None,
            region: "us-east-1",
            service: "iam",
            method: "GET",
            url: "https://iam.amazonaws.com/?Action=ListUsers&Version=2010-05-08",
            headers: &[],
            body: b"",
            amz_date: "20150830T123600Z",
        };
        let a = sign(inputs.clone());
        let b = sign(inputs);
        let auth_a = a
            .headers
            .iter()
            .find(|(k, _)| k == "authorization")
            .map(|(_, v)| v.as_str())
            .unwrap();
        let auth_b = b
            .headers
            .iter()
            .find(|(k, _)| k == "authorization")
            .map(|(_, v)| v.as_str())
            .unwrap();
        assert_eq!(auth_a, auth_b);
        assert!(auth_a.starts_with(
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/iam/aws4_request"
        ));
    }

    /// Signing-key derivation matches the AWS-published kSigning bytes
    /// for the canonical example (known good independent of canonical-
    /// request shape).
    #[test]
    fn signing_key_derivation_matches_aws_example() {
        // From https://docs.aws.amazon.com/general/latest/gr/signature-v4-examples.html
        // Inputs: secret=wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY
        //         date=20120215
        //         region=us-east-1
        //         service=iam
        // Expected kSigning hex:
        //   f4780e2d9f65fa895f9c67b32ce1baf0b0d8a43505a000a1a9e090d414db404d
        let secret = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";
        let k_date = hmac(format!("AWS4{secret}").as_bytes(), b"20120215");
        let k_region = hmac(&k_date, b"us-east-1");
        let k_service = hmac(&k_region, b"iam");
        let k_signing = hmac(&k_service, b"aws4_request");
        assert_eq!(
            hex(&k_signing),
            "f4780e2d9f65fa895f9c67b32ce1baf0b0d8a43505a000a1a9e090d414db404d"
        );
    }

    #[test]
    fn body_hash_round_trips() {
        let signed = sign(SigV4Input {
            access_key_id: "AK",
            secret_access_key: "secret",
            session_token: None,
            region: "us-east-1",
            service: "s3",
            method: "PUT",
            url: "https://bucket.s3.us-east-1.amazonaws.com/key",
            headers: &[],
            body: b"hello",
            amz_date: "20240101T000000Z",
        });
        let body_hash = signed
            .headers
            .iter()
            .find(|(k, _)| k == "x-amz-content-sha256")
            .map(|(_, v)| v.as_str())
            .unwrap();
        // SHA-256("hello")
        assert_eq!(
            body_hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn session_token_included_in_signed_headers() {
        let signed = sign(SigV4Input {
            access_key_id: "AK",
            secret_access_key: "secret",
            session_token: Some("session-token-value"),
            region: "us-east-1",
            service: "kms",
            method: "POST",
            url: "https://kms.us-east-1.amazonaws.com/",
            headers: &[("content-type", "application/x-amz-json-1.1")],
            body: b"{}",
            amz_date: "20240101T000000Z",
        });
        assert!(signed
            .headers
            .iter()
            .any(|(k, v)| k == "x-amz-security-token" && v == "session-token-value"));
        let auth = signed
            .headers
            .iter()
            .find(|(k, _)| k == "authorization")
            .map(|(_, v)| v.as_str())
            .unwrap();
        // SignedHeaders must include x-amz-security-token alphabetically.
        assert!(auth.contains("SignedHeaders=content-type;host;x-amz-content-sha256;x-amz-date;x-amz-security-token"));
    }
}
