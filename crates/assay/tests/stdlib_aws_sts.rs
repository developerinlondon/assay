mod common;

use common::run_lua;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// STS answers the query protocol in JSON when the Accept header asks for it —
// Expiration arrives as epoch seconds. These mocks mirror that shape.

fn sts_json(result_key: &str, access_key: &str) -> serde_json::Value {
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

#[tokio::test]
async fn test_parse_ini() {
    let script = r#"
        local sts = require("assay.aws.sts")
        local sections = sts._parse_ini([[
# a comment
[default]
aws_access_key_id = AKIDDEFAULT
aws_secret_access_key = sekret

[profile qa]
role_arn = arn:aws:iam::111122223333:role/qa-read
source_profile = default
region = eu-west-1
]])
        assert.eq(sections["default"].aws_access_key_id, "AKIDDEFAULT")
        assert.eq(sections["default"].aws_secret_access_key, "sekret")
        assert.eq(sections["profile qa"].role_arn, "arn:aws:iam::111122223333:role/qa-read")
        assert.eq(sections["profile qa"].source_profile, "default")
        assert.eq(sections["profile qa"].region, "eu-west-1")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_credentials_explicit_passthrough() {
    let script = r#"
        local sts = require("assay.aws.sts")
        local c = sts.credentials({ access_key = "AKID", secret_key = "sec", session_token = "tok" })
        assert.eq(c.access_key, "AKID")
        assert.eq(c.secret_key, "sec")
        assert.eq(c.session_token, "tok")
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_credentials_static_profile_from_files() {
    let dir = tempfile::tempdir().unwrap();
    let creds_path = dir.path().join("credentials");
    std::fs::write(
        &creds_path,
        "[myprofile]\naws_access_key_id = AKIDPROF\naws_secret_access_key = profsecret\n",
    )
    .unwrap();

    let script = format!(
        r#"
        local sts = require("assay.aws.sts")
        local c = sts.credentials({{
            profile = "myprofile",
            credentials_file = "{creds}",
            config_file = "/nonexistent-config",
        }})
        assert.eq(c.access_key, "AKIDPROF")
        assert.eq(c.secret_key, "profsecret")
        "#,
        creds = creds_path.display()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_credentials_profile_role_chain_via_assume_role() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .and(body_string_contains("Action=AssumeRole"))
        .and(body_string_contains(
            "RoleArn=arn%3Aaws%3Aiam%3A%3A111122223333%3Arole%2Fqa-read",
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(sts_json("AssumeRole", "ASIACHAINED")),
        )
        .mount(&server)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let creds_path = dir.path().join("credentials");
    std::fs::write(
        &creds_path,
        "[base]\naws_access_key_id = AKIDBASE\naws_secret_access_key = basesecret\n",
    )
    .unwrap();
    let config_path = dir.path().join("config");
    std::fs::write(
        &config_path,
        "[profile qa]\nrole_arn = arn:aws:iam::111122223333:role/qa-read\nsource_profile = base\n",
    )
    .unwrap();

    let script = format!(
        r#"
        local sts = require("assay.aws.sts")
        local c = sts.credentials({{
            profile = "qa",
            credentials_file = "{creds}",
            config_file = "{config}",
            sts_endpoint = "{sts}",
        }})
        assert.eq(c.access_key, "ASIACHAINED")
        assert.eq(c.secret_key, "resolved-secret")
        assert.eq(c.session_token, "resolved-session-token")
        "#,
        creds = creds_path.display(),
        config = config_path.display(),
        sts = server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_assume_role_with_web_identity() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .and(body_string_contains("Action=AssumeRoleWithWebIdentity"))
        .and(body_string_contains("WebIdentityToken=oidc-token-body"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(sts_json("AssumeRoleWithWebIdentity", "ASIAWEBID")),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local sts = require("assay.aws.sts")
        local c = sts.assume_role_with_web_identity(
            "arn:aws:iam::111122223333:role/irsa-role",
            "oidc-token-body",
            {{ sts_endpoint = "{sts}" }}
        )
        assert.eq(c.access_key, "ASIAWEBID")
        assert.eq(c.session_token, "resolved-session-token")
        "#,
        sts = server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_profile_web_identity_token_file() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .and(body_string_contains("Action=AssumeRoleWithWebIdentity"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(sts_json("AssumeRoleWithWebIdentity", "ASIAFROMFILE")),
        )
        .mount(&server)
        .await;

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

    let script = format!(
        r#"
        local sts = require("assay.aws.sts")
        local c = sts.credentials({{
            profile = "irsa",
            credentials_file = "/nonexistent-credentials",
            config_file = "{config}",
            sts_endpoint = "{sts}",
        }})
        assert.eq(c.access_key, "ASIAFROMFILE")
        "#,
        config = config_path.display(),
        sts = server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_missing_profile_errors_clearly() {
    let script = r#"
        local sts = require("assay.aws.sts")
        local ok, err = pcall(sts.credentials, {
            profile = "ghost",
            credentials_file = "/nonexistent",
            config_file = "/nonexistent",
        })
        assert.eq(ok, false)
        assert.contains(tostring(err), "profile 'ghost' not found")
    "#;
    run_lua(script).await.unwrap();
}
