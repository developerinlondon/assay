//! Phase-3 integration test.
//!
//! Spawns the `assay-engine` binary against a temp SQLite DB on a random
//! free port, polls `/api/v1/engine/workflow/health` until ready, then exercises the key
//! endpoints: health, version, namespaces, dashboard index. Confirms
//! shape of the responses.
//!
//! This test proves the whole binary wires together correctly — config
//! parsing → backend connect → migrations → axum compose → router
//! serving → both workflow API and dashboard paths answering.

use std::io::Write;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Find a free TCP port by binding to 127.0.0.1:0, reading the assigned
/// port, and dropping the listener. Race-prone in theory; fine for tests
/// that spawn immediately.
fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
    l.local_addr().unwrap().port()
}

fn engine_binary() -> PathBuf {
    // cargo test sets CARGO_BIN_EXE_<name> for integration tests.
    env!("CARGO_BIN_EXE_assay-engine").into()
}

struct EngineProcess {
    child: Child,
    port: u16,
    _tmpdir: tempfile::TempDir,
}

impl EngineProcess {
    fn spawn() -> Self {
        let port = free_port();
        let tmp = tempfile::tempdir().expect("tempdir");
        let db_path = tmp.path().join("engine.db");
        let cfg_path = tmp.path().join("engine.toml");

        let cfg = format!(
            r#"
[server]
bind_addr = "127.0.0.1:{port}"

[backend]
type = "sqlite"
path = "{db}"

[auth]
admin_api_keys = ["engine-smoke-test-key"]

[vault]
# KEK sealing (#113): the smoke test exercises the real sealed-at-rest
# path with an inline base64 32-byte unseal key. Boot fails closed
# without this (or dev_plaintext_kek), which is the whole point of the
# change. `AAAA...` decodes to 32 zero bytes — fine for a test key.
unseal_key_source = "base64:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="

[logging]
level = "error"
format = "pretty"
"#,
            db = db_path.display()
        );
        std::fs::write(&cfg_path, cfg).expect("write cfg");

        let child = Command::new(engine_binary())
            .arg("serve")
            .arg("--config")
            .arg(&cfg_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn engine");

        Self {
            child,
            port,
            _tmpdir: tmp,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }

    async fn wait_ready(&self, client: &reqwest::Client) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Ok(r) = client
                .get(self.url("/api/v1/engine/workflow/health"))
                .send()
                .await
                && r.status().is_success()
            {
                return;
            }
            if Instant::now() >= deadline {
                panic!("engine did not become ready on port {}", self.port);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

impl Drop for EngineProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn engine_smoke_sqlite() {
    // Log to stderr so test output shows the port if anything fails.
    let _ = writeln!(std::io::stderr(), "engine_smoke_sqlite starting");

    let engine = EngineProcess::spawn();
    let client = reqwest::Client::builder()
        // 30s — generous: Argon2id (m=64 MiB, t=3, p=4) on a slow CI
        // runner can take 2-3s per hash; the BW register/verify path
        // does multiple. 5s is too tight on ubuntu-latest 4-vCPU.
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    engine.wait_ready(&client).await;

    // ── /api/v1/engine/workflow/health ────────────────────────────────────────────────
    let r = client
        .get(engine.url("/api/v1/engine/workflow/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "health should return 200");
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["service"], "assay-workflow");
    assert_eq!(body["status"], "ok");

    // ── /api/v1/engine/workflow/version ───────────────────────────────────────────────
    let r = client
        .get(engine.url("/api/v1/engine/workflow/version"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert!(
        body.get("version").is_some(),
        "version response should have `version` field"
    );

    // ── /api/v1/engine/workflow/namespaces ────────────────────────────────────────────
    // Gated by the engine's auth layer — admin api-key
    // break-glass authenticates the request.
    let r = client
        .get(engine.url("/api/v1/engine/workflow/namespaces"))
        .header("Authorization", "Bearer engine-smoke-test-key")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    let arr = body.as_array().expect("namespaces should be a JSON array");
    assert!(
        arr.iter().any(|n| n["name"] == "main"),
        "`main` namespace should be auto-seeded on first connect"
    );

    // ── /workflow/ (dashboard index) ──────────────────────────────────
    let r = client.get(engine.url("/workflow/")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let ct = r
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        ct.starts_with("text/html"),
        "dashboard should return text/html, got {ct}"
    );
    let body = r.text().await.unwrap();
    assert!(
        body.contains("<html") || body.contains("<!DOCTYPE") || body.contains("<!doctype"),
        "dashboard body should contain HTML doctype"
    );

    // ── / (root → redirect to /workflow/) ─────────────────────────────
    let r = client.get(engine.url("/")).send().await.unwrap();
    assert!(
        r.status().is_success() || r.status().is_redirection(),
        "root should be 2xx or 3xx (redirect), got {}",
        r.status()
    );

    // ── /api/v1/vault/kv/* ────────────────────────────────────────────
    // Plan 17 / v0.3.0 vault module. Admin-key gated for Phase 1.
    let admin_bearer = "Bearer engine-smoke-test-key";

    // Unauthenticated: 401.
    let r = client
        .put(engine.url("/api/v1/vault/kv/api/stripe"))
        .json(&serde_json::json!({ "data": "sk_live_xxx" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401, "vault must reject missing bearer");

    // PUT with admin key.
    let r = client
        .put(engine.url("/api/v1/vault/kv/api/stripe"))
        .header("Authorization", admin_bearer)
        .json(&serde_json::json!({ "data": "sk_live_xxx" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201, "vault PUT should return 201; body: ?");
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["version"], 1);

    // GET round-trip.
    let r = client
        .get(engine.url("/api/v1/vault/kv/api/stripe"))
        .header("Authorization", admin_bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["data"], "sk_live_xxx");
    assert_eq!(body["version"], 1);

    // PUT another version, confirm GET returns the newer one.
    let r = client
        .put(engine.url("/api/v1/vault/kv/api/stripe"))
        .header("Authorization", admin_bearer)
        .json(&serde_json::json!({ "data": "sk_live_yyy" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let r = client
        .get(engine.url("/api/v1/vault/kv/api/stripe"))
        .header("Authorization", admin_bearer)
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["data"], "sk_live_yyy");
    assert_eq!(body["version"], 2);

    // ── /api/v1/vault/transit/* ───────────────────────────────────────
    let r = client
        .post(engine.url("/api/v1/vault/transit/keys/logs"))
        .header("Authorization", admin_bearer)
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201, "transit create should return 201");

    // Encrypt + decrypt round-trip.
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD as B64;
    let plaintext = b"hello-from-engine-smoke";
    let r = client
        .post(engine.url("/api/v1/vault/transit/encrypt/logs"))
        .header("Authorization", admin_bearer)
        .json(&serde_json::json!({
            "plaintext_b64": B64.encode(plaintext),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let ct: serde_json::Value = r.json().await.unwrap();
    let ciphertext = ct["ciphertext"].as_str().unwrap();
    assert!(ciphertext.starts_with("vault:v1:"));

    let r = client
        .post(engine.url("/api/v1/vault/transit/decrypt/logs"))
        .header("Authorization", admin_bearer)
        .json(&serde_json::json!({ "ciphertext": ciphertext }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    let decoded = B64
        .decode(body["plaintext_b64"].as_str().unwrap().as_bytes())
        .unwrap();
    assert_eq!(decoded, plaintext);

    // ── /api/v1/vault/sys/seal-status ─────────────────────────────────
    // KEK sealing (#113): the KEK was sealed at rest under the inline
    // unseal key and unsealed into memory at boot, so the vault is
    // operational (`sealed = false`) and reports method `sealed-v1`
    // (NOT plaintext — that would mean the KEK is unsealed on disk).
    let r = client
        .get(engine.url("/api/v1/vault/sys/seal-status"))
        .header("Authorization", admin_bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["sealed"], false);
    assert_eq!(body["method"], "sealed-v1");

    // ── /api/v1/vault/sys/seal — fail-closed semantics ────────────────
    let r = client
        .post(engine.url("/api/v1/vault/sys/seal"))
        .header("Authorization", admin_bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204, "seal should return 204");

    // After sealing, KV + transit ops must surface 503 / Sealed.
    let r = client
        .put(engine.url("/api/v1/vault/kv/api/post-seal"))
        .header("Authorization", admin_bearer)
        .json(&serde_json::json!({ "data": "should-be-rejected" }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        r.status(),
        503,
        "PUT after seal must fail-closed with 503; got {}",
        r.status()
    );
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["error"], "sealed");

    let r = client
        .post(engine.url("/api/v1/vault/transit/encrypt/logs"))
        .header("Authorization", admin_bearer)
        .json(&serde_json::json!({ "plaintext_b64": "Zm9v" }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        r.status(),
        503,
        "transit encrypt after seal must fail-closed; got {}",
        r.status()
    );

    // Status should now report sealed = true.
    let r = client
        .get(engine.url("/api/v1/vault/sys/seal-status"))
        .header("Authorization", admin_bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["sealed"], true);

    // ── Spawn a second engine for collection / personal vault flow ────
    // (the prior instance is sealed for the rest of this test). The
    // flow is admin-key gated; this proves the Phase-3 routes exist and
    // round-trip through the full PG/SQLite path.
    drop(engine);
    let engine2 = EngineProcess::spawn();
    let client2 = reqwest::Client::builder()
        // 30s — generous: Argon2id (m=64 MiB, t=3, p=4) on a slow CI
        // runner can take 2-3s per hash; the BW register/verify path
        // does multiple. 5s is too tight on ubuntu-latest 4-vCPU.
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();
    engine2.wait_ready(&client2).await;

    // Personal vault: ensure for user "alice" with a 32-byte X25519
    // pubkey (placeholder — real value comes from the auth crate).
    let pubkey_b64 = B64.encode([7u8; 32]);
    let r = client2
        .post(engine2.url("/api/v1/vault/me/alice"))
        .header("Authorization", admin_bearer)
        .json(&serde_json::json!({ "public_key_b64": pubkey_b64 }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["owner_user"], "alice");

    // Idempotent ensure — second call returns the same row.
    let r = client2
        .post(engine2.url("/api/v1/vault/me/alice"))
        .header("Authorization", admin_bearer)
        .json(&serde_json::json!({ "public_key_b64": pubkey_b64 }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let again: serde_json::Value = r.json().await.unwrap();
    assert_eq!(again["id"], body["id"], "ensure_vault must be idempotent");

    // Personal item — pre-encrypted bytes (the server is just a blob
    // store at this layer).
    let r = client2
        .post(engine2.url("/api/v1/vault/me/alice/items"))
        .header("Authorization", admin_bearer)
        .json(&serde_json::json!({
            "item_type": "login",
            "name": "github",
            "ciphertext_b64": B64.encode(b"encrypted-payload"),
            "nonce_b64": B64.encode([1u8; 12]),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["item_type"], "login");
    assert_eq!(body["name"], "github");

    // List personal items — should include the one we just created.
    let r = client2
        .get(engine2.url("/api/v1/vault/me/alice/items"))
        .header("Authorization", admin_bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["items"].as_array().unwrap().len(), 1);

    // Collection — create + add member + add item + list.
    let r = client2
        .post(engine2.url("/api/v1/vault/collections"))
        .header("Authorization", admin_bearer)
        .json(&serde_json::json!({
            "org_id": "org-acme",
            "name": "Engineering",
            "created_by": "alice",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let col: serde_json::Value = r.json().await.unwrap();
    let col_id = col["id"].as_str().unwrap().to_string();

    // Add member with wrapped collection key.
    let r = client2
        .post(engine2.url(&format!("/api/v1/vault/collections/{col_id}/members")))
        .header("Authorization", admin_bearer)
        .json(&serde_json::json!({
            "user_id": "alice",
            "wrapped_key_b64": B64.encode(b"wrapped-collection-key-32-bytes-here"),
            "role": "admin",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);

    // List members.
    let r = client2
        .get(engine2.url(&format!("/api/v1/vault/collections/{col_id}/members")))
        .header("Authorization", admin_bearer)
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["members"].as_array().unwrap().len(), 1);
    assert_eq!(body["members"][0]["user_id"], "alice");
    assert_eq!(body["members"][0]["role"], "admin");

    // Add item to the collection.
    let r = client2
        .post(engine2.url(&format!("/api/v1/vault/collections/{col_id}/items")))
        .header("Authorization", admin_bearer)
        .json(&serde_json::json!({
            "item_type": "login",
            "name": "shared-aws-root",
            "ciphertext_b64": B64.encode(b"shared-encrypted"),
            "nonce_b64": B64.encode([2u8; 12]),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);

    // List collection items.
    let r = client2
        .get(engine2.url(&format!("/api/v1/vault/collections/{col_id}/items")))
        .header("Authorization", admin_bearer)
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["items"][0]["name"], "shared-aws-root");

    // ── /api/v1/vault/share — biscuit share links (Phase 4) ──────────
    let r = client2
        .post(engine2.url("/api/v1/vault/share"))
        .header("Authorization", admin_bearer)
        .json(&serde_json::json!({
            "target_kind": "collection",
            "target_id": col_id,
            "ttl_secs": 60,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let mint: serde_json::Value = r.json().await.unwrap();
    let token = mint["token"].as_str().unwrap().to_string();
    let revocation_id = mint["revocation_ids"][0].as_str().unwrap().to_string();
    assert!(!token.is_empty());

    // Redeem — public surface, no admin gate.
    let r = client2
        .get(engine2.url(&format!("/api/v1/vault/share/{token}")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let grant: serde_json::Value = r.json().await.unwrap();
    assert_eq!(grant["target_kind"], "collection");
    assert_eq!(grant["target_id"], col_id);

    // Revoke and re-redeem — should now 403.
    let r = client2
        .post(engine2.url("/api/v1/vault/share/revoke"))
        .header("Authorization", admin_bearer)
        .json(&serde_json::json!({
            "revocation_id": revocation_id,
            "reason": "test-revoke",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let r = client2
        .get(engine2.url(&format!("/api/v1/vault/share/{token}")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403, "redeeming a revoked token must 403");

    // ── BW-compat shim (Phase 7) ─────────────────────────────────────
    // Discovery endpoints are public + unauthenticated.
    let r = client2.get(engine2.url("/api/alive")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["service"], "assay-vault");

    let r = client2
        .get(engine2.url("/api/version"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);

    let r = client2
        .get(engine2.url("/api/config"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["server"]["name"], "assay-vault");

    // ── BW prelogin → register → connect/token → sync round-trip ─────
    // Plan §S6: stock BW client flow. Drives the same routes `bw
    // login` would call.
    let bw_email = "bw-smoke@example.com";
    let bw_password = "0123456789abcdef0123456789abcdef"; // simulated KDF-derived hash

    // 1. Prelogin returns Argon2id KDF posture for the email.
    let r = client2
        .post(engine2.url("/api/accounts/prelogin"))
        .json(&serde_json::json!({ "email": bw_email }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["Kdf"], 1);
    assert_eq!(body["KdfIterations"], 3);
    assert_eq!(body["KdfMemory"], 64);

    // 2. Register the user.
    let r = client2
        .post(engine2.url("/api/accounts/register"))
        .json(&serde_json::json!({
            "Email": bw_email,
            "Name": "Smoke User",
            "MasterPasswordHash": bw_password,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);

    // 3. Re-register the same email → 400.
    let r = client2
        .post(engine2.url("/api/accounts/register"))
        .json(&serde_json::json!({
            "Email": bw_email,
            "MasterPasswordHash": bw_password,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400, "duplicate register should 400");

    // 4. /identity/connect/token — password grant returns a JWT.
    let r = client2
        .post(engine2.url("/identity/connect/token"))
        .form(&[
            ("grant_type", "password"),
            ("username", bw_email),
            ("password", bw_password),
            ("scope", "api offline_access"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(
        r.status(),
        200,
        "BW token grant should succeed for seeded user"
    );
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["Kdf"], 1, "issued JWT response carries Argon2id KDF");
    assert!(body["access_token"].as_str().unwrap_or("").len() > 20);
    let bw_jwt = body["access_token"].as_str().unwrap().to_string();

    // 5. Wrong password → 400 invalid_grant.
    let r = client2
        .post(engine2.url("/identity/connect/token"))
        .form(&[
            ("grant_type", "password"),
            ("username", bw_email),
            ("password", "definitely-wrong"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["error"], "invalid_grant");

    // 6. /api/sync with the JWT returns BW shape (auto-create vault row).
    let r = client2
        .get(engine2.url("/api/sync"))
        .bearer_auth(&bw_jwt)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["Object"], "sync");
    assert_eq!(body["Profile"]["Email"], bw_email);

    // 7. Create a passkey-bearing cipher (plan §S6 — passkey-as-cipher).
    let cipher_body = serde_json::json!({
        "Type": 1,
        "Name": "github",
        "Notes": "encrypted-notes-blob",
        "Login": {
            "Username": "alice@example.com",
            "Password": "encrypted-password-blob",
            "Uri": "https://github.com",
            "Fido2Credentials": [
                {
                    "CredentialId": "encrypted-credential-id",
                    "KeyType": "encrypted-key-type",
                    "KeyAlgorithm": "encrypted-alg",
                    "RpId": "encrypted-rpid",
                    "UserName": "encrypted-user",
                    "Counter": "encrypted-counter"
                }
            ]
        }
    });
    let r = client2
        .post(engine2.url("/api/ciphers"))
        .bearer_auth(&bw_jwt)
        .json(&cipher_body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let body: serde_json::Value = r.json().await.unwrap();
    let cipher_id = body["Id"].as_str().unwrap().to_string();
    assert_eq!(body["Type"], 1);
    assert!(body["Login"]["Fido2Credentials"].is_array());

    // 8. Sync now returns the cipher with passkey credentials intact.
    let r = client2
        .get(engine2.url("/api/sync"))
        .bearer_auth(&bw_jwt)
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = r.json().await.unwrap();
    let ciphers = body["Ciphers"].as_array().unwrap();
    assert_eq!(ciphers.len(), 1);
    assert_eq!(
        ciphers[0]["Login"]["Fido2Credentials"][0]["CredentialId"],
        "encrypted-credential-id"
    );

    // 9. Delete the cipher.
    let r = client2
        .delete(engine2.url(&format!("/api/ciphers/{cipher_id}")))
        .bearer_auth(&bw_jwt)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);

    // ── /vault/console — Phase-7 dashboard pane ──────────────────────
    let r = client2
        .get(engine2.url("/vault/console"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let ct = r
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        ct.starts_with("text/html"),
        "vault console should return text/html, got {ct}"
    );
    let body = r.text().await.unwrap();
    assert!(body.contains("Assay Vault"));
    // Pane controllers reference the documented endpoints.
    let r = client2
        .get(engine2.url("/vault/app.js"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body = r.text().await.unwrap();
    assert!(body.contains("/sys/seal-status"));
    assert!(body.contains("/transit/keys"));
    assert!(body.contains("/dynamic/leases"));
    // Plan §S10 — My vault + Collections panes (A7).
    assert!(body.contains("/me/"));
    assert!(body.contains("/collections"));
}
