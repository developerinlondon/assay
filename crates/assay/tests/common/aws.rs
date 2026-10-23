//! Fixtures shared by the `assay.aws.*` test binaries.

#![allow(dead_code)]

/// STS answers the query protocol in JSON when the Accept header asks for it —
/// Expiration arrives as epoch seconds. These mocks mirror that shape.
pub fn sts_json(result_key: &str, access_key: &str) -> serde_json::Value {
    serde_json::json!({
        format!("{result_key}Response"): {
            format!("{result_key}Result"): {
                "Credentials": {
                    "AccessKeyId": access_key,
                    "SecretAccessKey": "resolved-secret",
                    "SessionToken": "resolved-session-token",
                    "Expiration": 9999999999u64,
                }
            }
        }
    })
}

/// An IRSA-shaped credential source with no process-wide environment: a profile
/// named `irsa` carrying `role_arn` + `web_identity_token_file`. Resolving it
/// runs the same `AssumeRoleWithWebIdentity` POST an in-cluster pod would,
/// without the env-var mutation that would force tests to serialise on a lock.
pub struct WebIdentityProfile {
    _dir: tempfile::TempDir,
    pub config_path: std::path::PathBuf,
}

pub fn web_identity_profile() -> WebIdentityProfile {
    let dir = tempfile::tempdir().unwrap();
    let token_path = dir.path().join("oidc-token");
    std::fs::write(&token_path, "token-from-file").unwrap();
    let config_path = dir.path().join("config");
    std::fs::write(
        &config_path,
        format!(
            "[profile irsa]\nrole_arn = arn:aws:iam::111122223333:role/irsa\nweb_identity_token_file = {}\n",
            token_path.display()
        ),
    )
    .unwrap();
    WebIdentityProfile {
        _dir: dir,
        config_path,
    }
}
