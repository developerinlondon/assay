//! What an AWS API request actually does, independent of its HTTP verb.
//!
//! AWS puts reads and writes on the same verb in both directions. Resolving
//! credentials (`AssumeRole`, `AssumeRoleWithWebIdentity`) and minting an ECR
//! registry password are `POST`s that change nothing, so classifying by verb
//! refuses a readonly script its credentials before it issues a single
//! request. The EC2 Query API accepts `?Action=TerminateInstances` on a `GET`,
//! so classifying by verb *admits* a termination. This module reads the
//! request instead: the service from the SigV4 credential scope, the action
//! from `X-Amz-Target`, the form body, or the query string.
//!
//! A signature is a claim, not a capability: the service it names counts only
//! on a host that service owns, or any script could write an `Authorization`
//! header saying `sts` and carry the read exemption to an API Gateway stage.
//!
//! The answer is deliberately `None` for anything not recognisably an AWS API
//! request, so the gates fall back to the verb and nothing else moves. That
//! covers most of `amazonaws.com`, which hosts far more than AWS's own APIs.
//!
//! Header *values* are read here — the signature and the target live in them —
//! and nothing leaves but a `Classification` and, for a refusal, an action
//! name vetted to be a plain identifier.

use crate::lua::policy::Classification;

use super::HttpCall;

/// Enumerated rather than a `Get*`/`List*` prefix rule. `GetFederationToken`
/// and `GetSessionToken` mint credentials for someone else to use, which is
/// not a read however the name reads; a prefix rule would admit both. The EC2
/// entries are exactly what `stdlib/aws/ec2.lua` sends.
const READ_ACTIONS: &[(&str, &str)] = &[
    ("sts", "AssumeRole"),
    ("sts", "AssumeRoleWithWebIdentity"),
    ("sts", "GetCallerIdentity"),
    ("ecr", "GetAuthorizationToken"),
    ("ec2", "DescribeInstances"),
    ("ec2", "DescribeVolumes"),
    ("ec2", "DescribeSecurityGroups"),
];

/// Services whose operation is the method and the path, not an `Action`
/// parameter. S3 is the only one `assay.aws.*` reaches, and `s3.lua` reads
/// with `GET` throughout; a signed `POST /bucket?delete` is not covered and
/// stays a write.
const REST_READ_SERVICES: &[&str] = &["s3"];

/// Suffixes that make a host an AWS API endpoint. Matched on label
/// boundaries, which is what keeps `evilamazonaws.com` out.
const AWS_HOST_SUFFIXES: &[&str] = &[
    "amazonaws.com",
    "amazonaws.com.cn",
    "amazonaws.eu",
    "api.aws",
    "c2s.ic.gov",
    "sc2s.sgov.gov",
];

/// Services whose own hosts are AWS API endpoints, so a request there is
/// decided on its action whether or not its signature can be read. The Query
/// protocol ones take `?Action=` on a `GET`; `ecr` and `s3` are the others
/// `assay.aws.*` reaches. A host serving none of these — an API Gateway stage,
/// a load balancer — is the customer's, and its verb decides.
const API_SERVICES: &[&str] = &[
    "autoscaling",
    "cloudformation",
    "cloudsearch",
    "ec2",
    "ecr",
    "elasticache",
    "elasticbeanstalk",
    "elasticloadbalancing",
    "email",
    "iam",
    "importexport",
    "monitoring",
    "rds",
    "redshift",
    "s3",
    "sdb",
    "sns",
    "sqs",
    "sts",
];

/// Query parameters that are AWS authentication wherever they appear.
const PRESIGN_PARAMS: &[&str] = &[
    "X-Amz-Algorithm",
    "X-Amz-Credential",
    "X-Amz-Signature",
    "X-Amz-SignedHeaders",
];

/// Signature Version 2 query authentication. The names are generic enough
/// that a customer's own API may use them, so they count only on a host an
/// AWS API service owns.
const SIGV2_PARAMS: &[&str] = &[
    "AWSAccessKeyId",
    "Signature",
    "SignatureMethod",
    "SignatureVersion",
];

/// Every component a SigV4 `Authorization` header carries, each exactly once.
const SIGV4_COMPONENTS: &[&str] = &["Credential", "SignedHeaders", "Signature"];

/// The one AWS call nobody can sign: exchanging a web identity token for
/// credentials is authenticated by the token itself, so it arrives unsigned
/// and only an STS host may serve it.
const WEB_IDENTITY_ACTION: &str = "AssumeRoleWithWebIdentity";
const STS_SERVICE: &str = "sts";

/// Labels AWS itself puts between a service and the suffix.
const ENDPOINT_QUALIFIERS: &[&str] = &["dualstack", "fips", "vpce"];

/// Older host labels a service still answers on.
const HOST_ALIASES: &[(&str, &str)] = &[("sqs", "queue")];

/// The one signing scope honoured off AWS's own hosts: S3-compatible stores
/// (MinIO and the like) speak SigV4 for `s3`, and their reads stay reads.
const S3_SERVICE: &str = "s3";

const SIGV4_ALGORITHM: &str = "AWS4-HMAC-SHA256";

/// The longest action name that may appear in an error message.
const ACTION_NAME_CAP: usize = 64;

/// How this request was classified, and — for a write — enough to say why.
enum Verdict {
    Read,
    Write(Reason),
}

enum Reason {
    /// Two header names equal but for case. Only one value is sent, and the
    /// signature or target the classifier read must be the one that is.
    HeaderCollision,
    /// Two `X-Amz-Credential` parameters, so two credential scopes.
    DuplicateCredential,
    /// Authentication material this cannot read. What AWS would make of it is
    /// unknown, so it is never taken for an unsigned request.
    UnreadableSignature,
    /// A readable scope naming a service this AWS host does not serve.
    UnbackedScope,
    /// An `s3`-signed request that names an operation (`Action`,
    /// `X-Amz-Target`), which S3's own reads never do.
    S3Operation,
    /// AWS signing material on a host that is not an AWS name, such as an IP
    /// literal: nothing says which service it would reach.
    SignedForeignHost,
    /// Two `Action` parameters: which one the service would honour is its
    /// business, so neither is a basis for admitting the call.
    DuplicateAction,
    /// `X-Amz-Target` and `Action` name different operations.
    ConflictingAction,
    /// An action that is not on the read allowlist.
    NotAllowed { service: String, action: String },
    /// A signed request with no action at all, on a service where the verb
    /// and the path are the operation.
    NotARead { service: String, method: String },
}

/// `Read` or `Write` for a recognisable AWS API request, `None` when the
/// caller should fall back to the verb.
pub(crate) fn classify(call: &HttpCall) -> Option<Classification> {
    match verdict(call)? {
        Verdict::Read => Some(Classification::Read),
        Verdict::Write(_) => Some(Classification::Write),
    }
}

/// Why a refusal happened, for the error message. `None` when this request is
/// not an AWS write, so a caller refusing for some other reason says nothing
/// about AWS.
pub(crate) fn refusal_reason(call: &HttpCall) -> Option<String> {
    match verdict(call)? {
        Verdict::Read => None,
        Verdict::Write(reason) => Some(describe(&reason)),
    }
}

fn describe(reason: &Reason) -> String {
    match reason {
        Reason::HeaderCollision => "aws request names a header twice".to_string(),
        Reason::DuplicateCredential => {
            "aws request carries more than one X-Amz-Credential".to_string()
        }
        Reason::UnreadableSignature => {
            "aws request carries a signature that cannot be read".to_string()
        }
        Reason::UnbackedScope => {
            "aws request is signed for a service this host does not serve".to_string()
        }
        Reason::S3Operation => "aws s3-signed request names an operation".to_string(),
        Reason::SignedForeignHost => {
            "aws-signed request to a host that is not an AWS endpoint".to_string()
        }
        Reason::DuplicateAction => "aws request carries more than one Action".to_string(),
        Reason::ConflictingAction => "aws request's X-Amz-Target and Action disagree".to_string(),
        Reason::NotAllowed { service, action } => not_allowed(service, action),
        Reason::NotARead { service, method } => {
            format!("aws {service} {} is not a read", method.to_uppercase())
        }
    }
}

/// The action is named only when `identifier` vetted it; otherwise the message
/// says one was asked for without repeating it.
fn not_allowed(service: &str, action: &str) -> String {
    match identifier(action) {
        Some(action) => format!("aws {service} Action={action} is not a read"),
        None => format!("aws {service} action is not a read"),
    }
}

/// An action name reaches an error message only when it is plainly an
/// identifier and short: it is attacker-controlled text, and the message is
/// shown to a human and persisted.
fn identifier(value: &str) -> Option<&str> {
    let plain = !value.is_empty()
        && value.len() <= ACTION_NAME_CAP
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_');
    plain.then_some(value)
}

/// What authenticates a request to an AWS host, as far as it can be read.
enum Auth {
    Unsigned,
    /// The service a readable SigV4 credential scope names: a claim until the
    /// host is seen to back it.
    Scope(String),
    Unreadable,
}

fn verdict(call: &HttpCall) -> Option<Verdict> {
    let url = url::Url::parse(call.url.as_deref()?).ok()?;
    let Some(labels) = aws_host_labels(&url) else {
        return foreign_host_verdict(call, &url);
    };
    if call.headers_collide {
        return Some(Verdict::Write(Reason::HeaderCollision));
    }
    if query_values(&url, "X-Amz-Credential").len() > 1 {
        return Some(Verdict::Write(Reason::DuplicateCredential));
    }
    let api_service = API_SERVICES
        .iter()
        .copied()
        .find(|service| host_serves(&labels, service));
    match authentication(call, &url, api_service.is_some()) {
        Auth::Unreadable => Some(Verdict::Write(Reason::UnreadableSignature)),
        Auth::Scope(service) if host_serves(&labels, &service) => {
            Some(signed_verdict(call, &url, service))
        }
        Auth::Scope(service) if service == S3_SERVICE && serves_s3_elsewhere(&labels) => {
            Some(s3_read_verdict(call, &url))
        }
        Auth::Scope(_) => Some(Verdict::Write(Reason::UnbackedScope)),
        Auth::Unsigned => unsigned_verdict(call, &url, api_service?),
    }
}

/// A host that is not an AWS name can still be AWS: an IP literal, or a
/// wildcard-DNS alias such as `<ip>.sslip.io`, reaches an endpoint just the
/// same. With no AWS signing material the verb decides, as for any host.
/// With some, the service it would reach is unknowable, so it is a write —
/// unless it is a readable `s3` scope on a `GET`, an S3-compatible store.
fn foreign_host_verdict(call: &HttpCall, url: &url::Url) -> Option<Verdict> {
    if !carries_signing_material(call, url) {
        return None;
    }
    let s3_scope = matches!(authentication(call, url, true), Auth::Scope(s) if s == S3_SERVICE);
    if s3_scope && !call.headers_collide && matches!(s3_read_verdict(call, url), Verdict::Read) {
        return Some(Verdict::Read);
    }
    Some(Verdict::Write(Reason::SignedForeignHost))
}

/// An `s3`-signed request on a host S3 answers on without the `s3` label: a
/// `GET` or `HEAD` is a read unless it names an operation, which no S3 read
/// does and a Query-protocol mutation must.
fn s3_read_verdict(call: &HttpCall, url: &url::Url) -> Verdict {
    if names_operation(call, url) {
        return Verdict::Write(Reason::S3Operation);
    }
    if is_rest_read(S3_SERVICE, &call.method) {
        return Verdict::Read;
    }
    let method = call.method.clone();
    Verdict::Write(Reason::NotARead {
        service: S3_SERVICE.to_string(),
        method,
    })
}

fn names_operation(call: &HttpCall, url: &url::Url) -> bool {
    has_any_param(url, &["Action"]) || header(call, "x-amz-target").is_some()
}

/// S3 hosts whose service label is not `s3`: access points
/// (`<name>-<account>.s3-accesspoint.<region>`) and the legacy dash-region
/// endpoints (`s3-us-west-2`, `s3-external-1`), bucket-prefixed or not.
fn serves_s3_elsewhere(labels: &[String]) -> bool {
    labels.iter().enumerate().any(|(i, label)| {
        let legacy = label
            .strip_prefix("s3-")
            .is_some_and(|rest| rest == "external-1" || is_region(rest));
        let s3_label = label == "s3-accesspoint" || legacy;
        s3_label
            && labels[i + 1..]
                .iter()
                .all(|label| is_endpoint_qualifier(label))
    })
}

fn carries_signing_material(call: &HttpCall, url: &url::Url) -> bool {
    let header = header(call, "authorization").is_some_and(is_aws_scheme);
    let sigv2 = has_any_param(url, &["AWSAccessKeyId"]) && has_any_param(url, &["Signature"]);
    header || sigv2 || has_any_param(url, PRESIGN_PARAMS)
}

/// Signed by a service that owns this host, so the credential scope names what
/// is being asked and the action decides. An allowlisted action is a read and
/// nothing else is — which is the half that closes the mutating-`GET` hole.
fn signed_verdict(call: &HttpCall, url: &url::Url, service: String) -> Verdict {
    let action = match requested_action(call, url) {
        Ok(action) => action,
        Err(reason) => return Verdict::Write(reason),
    };
    match action {
        Some(action) => {
            if is_read_action(&service, &action) {
                Verdict::Read
            } else {
                Verdict::Write(Reason::NotAllowed { service, action })
            }
        }
        None => {
            if is_rest_read(&service, &call.method) {
                Verdict::Read
            } else {
                let method = call.method.clone();
                Verdict::Write(Reason::NotARead { service, method })
            }
        }
    }
}

/// Not signed for this host, on a host an AWS API service owns. Whether AWS
/// would honour the request is not this classifier's to guess, so the action
/// decides: anything off the read list is a write.
///
/// A read action unsigned is left to the verb — it changes nothing, and
/// without credentials AWS refuses it — except the web identity exchange,
/// which the OIDC token authenticates and so is always unsigned. S3 is REST,
/// with no action to read, so an unsigned object `GET` is the verb's too.
fn unsigned_verdict(call: &HttpCall, url: &url::Url, service: &str) -> Option<Verdict> {
    if REST_READ_SERVICES.contains(&service) {
        return None;
    }
    let action = match requested_action(call, url) {
        Ok(action) => action?,
        Err(reason) => return Some(Verdict::Write(reason)),
    };
    if service == STS_SERVICE && action == WEB_IDENTITY_ACTION {
        return Some(Verdict::Read);
    }
    if is_read_action(service, &action) {
        return None;
    }
    let service = service.to_string();
    Some(Verdict::Write(Reason::NotAllowed { service, action }))
}

/// An AWS API host, as the labels in front of the AWS suffix. `None` for
/// anything else: an IP literal is never one, so a signature pointed at an
/// address gets no special treatment, and the suffix has to start at a label
/// boundary, so `evilamazonaws.com` is somebody else's domain.
fn aws_host_labels(url: &url::Url) -> Option<Vec<String>> {
    let url::Host::Domain(host) = url.host()? else {
        return None;
    };
    let host = host.strip_suffix('.').unwrap_or(host).to_ascii_lowercase();
    for suffix in AWS_HOST_SUFFIXES {
        if host == *suffix {
            return Some(Vec::new());
        }
        if let Some(head) = strip_dotted_suffix(&host, suffix) {
            return Some(head.split('.').map(String::from).collect());
        }
    }
    None
}

fn strip_dotted_suffix<'a>(host: &'a str, suffix: &str) -> Option<&'a str> {
    host.strip_suffix(suffix)?.strip_suffix('.')
}

/// Whether the service a signature claims is the service that owns this host.
///
/// Without this check an `Authorization` header is a bare assertion: a script
/// could claim `sts` on any `amazonaws.com` host it can reach — an API Gateway
/// stage, a load balancer, an AppSync endpoint — none of which verify SigV4,
/// and carry the read exemption to a `POST` that mutates. A signature is not a
/// capability.
///
/// Everything after the service's own label is AWS's to assign: a region,
/// `dualstack`, `fips`, a `vpce` zone. So a host whose tail is
/// `sts.us-east-1` really is STS, however the name begins. Labels *before* it
/// are not constrained, because that is where the legitimate shapes vary — an
/// S3 bucket, ECR's `api.`, a VPC endpoint id.
fn host_serves(labels: &[String], service: &str) -> bool {
    let fips = format!("{service}-fips");
    let alias = HOST_ALIASES
        .iter()
        .find(|(name, _)| *name == service)
        .map(|(_, alias)| *alias);
    for (i, label) in labels.iter().enumerate() {
        let label = label.as_str();
        let names_service = label == service || label == fips || Some(label) == alias;
        if !names_service {
            continue;
        }
        let tail = &labels[i + 1..];
        if tail.iter().all(|label| is_endpoint_qualifier(label)) {
            return true;
        }
    }
    false
}

fn is_endpoint_qualifier(label: &str) -> bool {
    ENDPOINT_QUALIFIERS.contains(&label) || is_region(label)
}

/// `us-east-1`, `eu-west-3`, `cn-north-1`, `us-gov-west-1`, `eusc-de-east-1`,
/// `us-iso-east-1`. Deliberately not
/// a loose "looks like a word" match: `s3-us-east-1` must not pass it, or a
/// bucket named `sts` would make `sts.s3-us-east-1.amazonaws.com` an STS host.
fn is_region(label: &str) -> bool {
    let mut parts = label.split('-');
    let area = parts.next().unwrap_or_default();
    let rest: Vec<&str> = parts.collect();
    let Some(number) = rest.last() else {
        return false;
    };
    let area_ok = (2..=4).contains(&area.len()) && all_ascii_lowercase(area);
    let number_ok = !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit());
    area_ok && number_ok && rest.len() >= 2
}

fn all_ascii_lowercase(value: &str) -> bool {
    value.bytes().all(|b| b.is_ascii_lowercase())
}

fn is_read_action(service: &str, action: &str) -> bool {
    READ_ACTIONS
        .iter()
        .any(|(s, a)| *s == service && *a == action)
}

/// A read on a service whose operation is the method and the path rather than
/// an `Action` parameter.
fn is_rest_read(service: &str, method: &str) -> bool {
    REST_READ_SERVICES.contains(&service)
        && (method.eq_ignore_ascii_case("get") || method.eq_ignore_ascii_case("head"))
}

/// The authentication on a request to an AWS host. A header-signed and a
/// presigned request are each read by their named parts; anything AWS-shaped
/// that does not read cleanly, or two mechanisms at once, is `Unreadable`.
fn authentication(call: &HttpCall, url: &url::Url, api_host: bool) -> Auth {
    let header = header(call, "authorization").filter(|value| is_aws_scheme(value));
    let presigned = has_any_param(url, PRESIGN_PARAMS);
    let sigv2 = api_host && has_any_param(url, SIGV2_PARAMS);
    match (header, presigned, sigv2) {
        (None, false, false) => Auth::Unsigned,
        (Some(value), false, false) => header_scope(value),
        (None, true, false) => presigned_scope(url),
        _ => Auth::Unreadable,
    }
}

/// `AWS4-HMAC-SHA256`, SigV4a's `AWS4-ECDSA-P256-SHA256`, SigV2's `AWS ` (the
/// space included), in any case. Only the first is read; the rest are AWS's
/// and unreadable here. A scheme that merely starts with the letters, such as
/// `AWSomeToken`, is somebody else's.
fn is_aws_scheme(value: &str) -> bool {
    ["AWS4-", "AWS "].iter().any(|prefix| {
        value
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
    })
}

fn has_any_param(url: &url::Url, names: &[&str]) -> bool {
    url.query_pairs()
        .any(|(key, _)| names.iter().any(|name| key.eq_ignore_ascii_case(name)))
}

/// `AWS4-HMAC-SHA256 Credential=…, SignedHeaders=…, Signature=…`, read by
/// component rather than by searching for `Credential=`, so a decoy such as
/// `Foo=Credential=x` is an unknown component, not a second scope.
fn header_scope(value: &str) -> Auth {
    let Some((algorithm, rest)) = value.split_once(char::is_whitespace) else {
        return Auth::Unreadable;
    };
    if !algorithm.eq_ignore_ascii_case(SIGV4_ALGORITHM) {
        return Auth::Unreadable;
    }
    let mut components: Vec<(&str, &str)> = Vec::new();
    for component in rest.split(',') {
        let Some((name, value)) = component.trim().split_once('=') else {
            return Auth::Unreadable;
        };
        let known = SIGV4_COMPONENTS.contains(&name);
        if !known || value.is_empty() || components.iter().any(|(seen, _)| *seen == name) {
            return Auth::Unreadable;
        }
        components.push((name, value));
    }
    if components.len() != SIGV4_COMPONENTS.len() {
        return Auth::Unreadable;
    }
    match components.iter().find(|(name, _)| *name == "Credential") {
        Some((_, credential)) => scope(credential),
        None => Auth::Unreadable,
    }
}

fn presigned_scope(url: &url::Url) -> Auth {
    let algorithms = query_values(url, "X-Amz-Algorithm");
    let algorithm_ok = match algorithms.as_slice() {
        [] => true,
        [one] => one.eq_ignore_ascii_case(SIGV4_ALGORITHM),
        _ => false,
    };
    match query_values(url, "X-Amz-Credential").as_slice() {
        [credential] if algorithm_ok => scope(credential),
        _ => Auth::Unreadable,
    }
}

fn scope(credential: &str) -> Auth {
    service_of_scope(credential).map_or(Auth::Unreadable, Auth::Scope)
}

/// `AKID/20260101/us-east-1/ec2/aws4_request` → `ec2`.
fn service_of_scope(credential: &str) -> Option<String> {
    let parts: Vec<&str> = credential.split('/').collect();
    if parts.len() != 5 || parts[4] != "aws4_request" || parts[3].is_empty() {
        return None;
    }
    Some(parts[3].to_ascii_lowercase())
}

/// The one action this request asks for. `Err` when it names more than one, so
/// there is no single answer to classify.
fn requested_action(call: &HttpCall, url: &url::Url) -> Result<Option<String>, Reason> {
    let mut actions = query_values(url, "Action");
    if let Some(body) = &call.body {
        actions.extend(body_actions(body));
    }
    if actions.len() > 1 {
        return Err(Reason::DuplicateAction);
    }
    let target = header(call, "x-amz-target").map(target_operation);
    match (actions.pop(), target) {
        (Some(action), Some(target)) if action != target => Err(Reason::ConflictingAction),
        (Some(action), _) => Ok(Some(action)),
        (None, target) => Ok(target),
    }
}

fn query_values(url: &url::Url, name: &str) -> Vec<String> {
    url.query_pairs()
        .filter(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.into_owned())
        .collect()
}

/// The Query protocol puts the action in a form-encoded body. Parsing every
/// body this way costs nothing: a JSON body has no `Action` key to find.
fn body_actions(body: &str) -> Vec<String> {
    url::form_urlencoded::parse(body.as_bytes())
        .filter(|(key, _)| key.eq_ignore_ascii_case("Action"))
        .map(|(_, value)| value.into_owned())
        .collect()
}

/// `AmazonEC2ContainerRegistry_V20150921.GetAuthorizationToken` → the
/// operation is what follows the last dot.
fn target_operation(target: &str) -> String {
    match target.rsplit_once('.') {
        Some((_, operation)) => operation.to_string(),
        None => target.to_string(),
    }
}

fn header<'a>(call: &'a HttpCall, name: &str) -> Option<&'a str> {
    call.headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim())
}

#[cfg(test)]
mod tests;
