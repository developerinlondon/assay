//! Shared harness: a scratch Postgres database and a real engine process.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub fn engine_binary() -> PathBuf {
    env!("CARGO_BIN_EXE_assay-engine").into()
}

pub fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
    l.local_addr().unwrap().port()
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("build client")
}

/// A throwaway database on the server `TEST_DATABASE_URL` names. The
/// `assay_test_` prefix is the one CI's cleanup guard sweeps.
pub struct ScratchDb {
    admin_url: String,
    name: String,
}

impl ScratchDb {
    pub async fn create() -> Option<Self> {
        let admin_url = std::env::var("TEST_DATABASE_URL")
            .ok()
            .filter(|u| !u.is_empty())?;
        let name = format!(
            "assay_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let pool = sqlx::PgPool::connect(&admin_url).await.ok()?;
        sqlx::query(&format!(r#"CREATE DATABASE "{name}""#))
            .execute(&pool)
            .await
            .expect("create scratch database");
        pool.close().await;
        Some(Self { admin_url, name })
    }

    /// The scratch database's own URL: the admin URL's credentials and
    /// host with the database name swapped in.
    pub fn url(&self) -> String {
        let base = self.admin_url.split('?').next().unwrap_or(&self.admin_url);
        let cut = base.rfind('/').expect("database url has a path");
        format!("{}/{}", &base[..cut], self.name)
    }

    pub async fn pool(&self) -> sqlx::PgPool {
        sqlx::PgPool::connect(&self.url())
            .await
            .expect("connect scratch database")
    }
}

impl Drop for ScratchDb {
    fn drop(&mut self) {
        let admin_url = self.admin_url.clone();
        let name = self.name.clone();
        let _ = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("cleanup runtime");
            rt.block_on(async {
                let Ok(pool) = sqlx::PgPool::connect(&admin_url).await else {
                    return;
                };
                let _ = sqlx::query(
                    "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1",
                )
                .bind(&name)
                .execute(&pool)
                .await;
                let _ = sqlx::query(&format!(r#"DROP DATABASE IF EXISTS "{name}""#))
                    .execute(&pool)
                    .await;
                pool.close().await;
            });
        })
        .join();
    }
}

pub struct EngineProcess {
    child: Child,
    pub port: u16,
    stderr: PathBuf,
}

impl EngineProcess {
    /// Spawn `assay-engine serve` on a free port against `backend`, a
    /// rendered `[backend]` TOML section.
    pub fn spawn(dir: &Path, tag: &str, backend: &str) -> Self {
        Self::spawn_with_env(dir, tag, backend, None)
    }

    /// As [`Self::spawn`], with an optional vault seal key in the
    /// engine's environment.
    pub fn spawn_with_env(dir: &Path, tag: &str, backend: &str, seal_key: Option<&str>) -> Self {
        let port = free_port();
        let cfg_path = dir.join(format!("engine-{tag}.toml"));
        let stderr = dir.join(format!("engine-{tag}.log"));
        std::fs::write(
            &cfg_path,
            format!(
                r#"
[server]
bind_addr = "127.0.0.1:{port}"

{backend}

[auth]
admin_api_keys = ["{ADMIN_KEY}"]

[logging]
level = "info"
format = "pretty"
"#,
                ADMIN_KEY = ADMIN_KEY
            ),
        )
        .expect("write config");

        let log = std::fs::File::create(&stderr).expect("create log");
        let mut command = Command::new(engine_binary());
        command.arg("serve").arg("--config").arg(&cfg_path);
        match seal_key {
            Some(key) => command.env("ASSAY_VAULT_SEAL_KEY", key),
            None => command.env_remove("ASSAY_VAULT_SEAL_KEY"),
        };
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(log))
            .spawn()
            .expect("spawn engine");

        Self {
            child,
            port,
            stderr,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(&self.stderr).unwrap_or_default()
    }

    pub fn exited(&mut self) -> Option<std::process::ExitStatus> {
        self.child.try_wait().ok().flatten()
    }

    /// Poll health until the engine serves. Errs with the engine's log
    /// when it exits first or never answers.
    pub async fn wait_ready(&mut self, client: &reqwest::Client) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if let Ok(r) = client
                .get(self.url("/api/v1/engine/workflow/health"))
                .send()
                .await
                && r.status().is_success()
            {
                return Ok(());
            }
            if let Some(status) = self.exited() {
                return Err(format!("exited {status}: {}", self.log()));
            }
            if Instant::now() >= deadline {
                return Err(format!("never became ready: {}", self.log()));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Stop the engine and wait for the process to go, so its store is
    /// closed before anything else opens it.
    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for EngineProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

pub const ADMIN_KEY: &str = "store-migrate-test-key";
