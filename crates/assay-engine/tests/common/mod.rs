//! Shared harness: a scratch Postgres database and a real engine process.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub fn engine_binary() -> PathBuf {
    env!("CARGO_BIN_EXE_assay-engine").into()
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
    /// The port this engine really bound, learned from its own log.
    /// `None` until [`Self::wait_ready`] has read it.
    pub port: Option<u16>,
    stderr: PathBuf,
    stdout: PathBuf,
}

impl EngineProcess {
    /// Spawn `assay-engine serve` on a kernel-chosen port against
    /// `backend`, a rendered `[backend]` TOML section.
    pub fn spawn(dir: &Path, tag: &str, backend: &str) -> Self {
        Self::spawn_with_env(dir, tag, backend, None)
    }

    /// As [`Self::spawn`], with an optional vault seal key in the
    /// engine's environment.
    pub fn spawn_with_env(dir: &Path, tag: &str, backend: &str, seal_key: Option<&str>) -> Self {
        let cfg_path = dir.join(format!("engine-{tag}.toml"));
        let stderr = dir.join(format!("engine-{tag}.log"));
        let stdout = dir.join(format!("engine-{tag}.out"));
        // Port 0: the kernel picks, and the engine logs what it got.
        // Picking a port here instead would mean binding a probe socket
        // and dropping it, which returns the number to the ephemeral
        // pool before the child binds — two engines can then draw the
        // same one and each answer the other's health probe.
        std::fs::write(
            &cfg_path,
            format!(
                r#"
[server]
bind_addr = "127.0.0.1:0"

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

        let err_log = std::fs::File::create(&stderr).expect("create log");
        let out_log = std::fs::File::create(&stdout).expect("create stdout log");
        let mut command = Command::new(engine_binary());
        command.arg("serve").arg("--config").arg(&cfg_path);
        match seal_key {
            Some(key) => command.env("ASSAY_VAULT_SEAL_KEY", key),
            None => command.env_remove("ASSAY_VAULT_SEAL_KEY"),
        };
        let child = command
            // Readiness parses the child's `listening` line, so the
            // config's own level has to be the one that applies, and
            // the fields have to be free of colour escapes.
            .env_remove("RUST_LOG")
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::from(out_log))
            .stderr(Stdio::from(err_log))
            .spawn()
            .expect("spawn engine");

        Self {
            child,
            port: None,
            stderr,
            stdout,
        }
    }

    pub fn url(&self, path: &str) -> String {
        let port = self
            .port
            .expect("the engine's port is only known once wait_ready has succeeded");
        format!("http://127.0.0.1:{port}{path}")
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(&self.stderr).unwrap_or_default()
    }

    /// The engine's tracing output, which is where the `listening` line
    /// lands. Failures are printed with `eprintln!`, so [`Self::log`]
    /// stays the one to quote in assertions.
    fn tracing_log(&self) -> String {
        std::fs::read_to_string(&self.stdout).unwrap_or_default()
    }

    pub fn exited(&mut self) -> Option<std::process::ExitStatus> {
        self.child.try_wait().ok().flatten()
    }

    /// Wait for the engine to serve: first learn the port it bound, then
    /// poll health on it. Errs with the engine's log when it exits first
    /// or never answers.
    pub async fn wait_ready(&mut self, client: &reqwest::Client) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(120);
        let port = self.wait_for_port(deadline).await?;
        self.port = Some(port);

        loop {
            // Ask whether our own child is still alive before trusting
            // anything on the wire: a dead engine's port goes back to
            // the pool, and a stranger holding it must not read as
            // ready.
            if let Some(status) = self.exited() {
                return Err(format!("exited {status}: {}", self.log()));
            }
            if let Ok(r) = client
                .get(self.url("/api/v1/engine/workflow/health"))
                .send()
                .await
                && r.status().is_success()
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!("never became ready: {}", self.log()));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Read the bound port out of this child's own tracing output. An
    /// engine that dies before it binds is reported as the exit it was,
    /// without a single HTTP request — so a sibling engine's socket can
    /// never be mistaken for this one's.
    async fn wait_for_port(&mut self, deadline: Instant) -> Result<u16, String> {
        loop {
            if let Some(port) = listening_port(&self.tracing_log()) {
                return Ok(port);
            }
            if let Some(status) = self.exited() {
                return Err(format!("exited {status}: {}", self.log()));
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "never logged a listening address\n--- stdout ---\n{}--- stderr ---\n{}",
                    self.tracing_log(),
                    self.log()
                ));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
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

/// The port from `bind_and_serve`'s line, e.g.
/// `… INFO assay-engine: listening actual=127.0.0.1:34567`.
fn listening_port(log: &str) -> Option<u16> {
    log.split_inclusive('\n')
        // Only whole lines: the file is read while the child writes it,
        // and half of an address parses as a different port.
        .filter(|line| line.ends_with('\n') && line.contains("listening"))
        .find_map(|line| {
            let addr = line.split("actual=").nth(1)?.split_whitespace().next()?;
            addr.parse::<std::net::SocketAddr>().ok().map(|a| a.port())
        })
}

pub const ADMIN_KEY: &str = "store-migrate-test-key";

#[cfg(test)]
mod tests {
    use super::listening_port;

    const LINE: &str =
        "2026-09-15T05:24:00.123456Z  INFO assay-engine: listening actual=127.0.0.1:34567\n";

    #[test]
    fn reads_the_port_out_of_the_listening_line() {
        assert_eq!(listening_port(LINE), Some(34567));

        // The line the engine really writes is preceded by its boot log.
        let preamble = "2026-09-15T05:24:00.1Z  INFO assay-engine: opening store\n";
        assert_eq!(listening_port(&format!("{preamble}{LINE}")), Some(34567));
    }

    #[test]
    fn ignores_a_line_the_child_has_not_finished_writing() {
        // The last two digits and the newline have yet to be written,
        // so what is there parses as a valid — and wrong — port.
        let torn = LINE.trim_end().strip_suffix("67").expect("trim the port");
        assert!(torn.ends_with("127.0.0.1:345"), "precondition: {torn}");
        assert_eq!(listening_port(torn), None);
    }

    #[test]
    fn nothing_to_read_yet() {
        assert_eq!(listening_port(""), None);
        assert_eq!(listening_port("INFO assay-engine: booting store\n"), None);
    }
}
