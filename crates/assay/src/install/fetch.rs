//! Fetch + cache + sha256-verify for declared deps.
//!
//! Cache flow per dep:
//! 1. If `cache_path` exists and its sha256 matches `expected_sha256` → cache
//!    hit; return (zero HTTP).
//! 2. If `cache_path` exists with a wrong sha → drop it and fall through.
//! 3. If `offline` is true → return [`FetchError::OfflineMissing`].
//! 4. Otherwise: HTTPS GET → write to `<file>.tmp` → sha-verify → atomic
//!    rename to `cache_path`.

use std::io;
use std::path::{Path, PathBuf};

use data_encoding::HEXLOWER;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::fs as afs;
use tokio::io::AsyncWriteExt;

use super::manifest::{Extension, Lib};

const RELEASE_BASE: &str = "https://github.com/developerinlondon/assay/releases/download";

#[derive(Debug, Error)]
pub enum FetchError {
    #[error("{name}: no sha256 declared for arch `{arch}` (have: {available:?})")]
    NoArchHash {
        name: String,
        arch: String,
        available: Vec<String>,
    },

    #[error("{name}: HTTP request to {url} failed: {source}")]
    Http {
        name: String,
        url: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("{name}: HTTP {status} from {url}")]
    Status {
        name: String,
        url: String,
        status: u16,
    },

    #[error("{name}: I/O error: {source}")]
    Io {
        name: String,
        #[source]
        source: io::Error,
    },

    #[error("{name}: sha256 mismatch (expected {expected}, got {actual})")]
    Sha256Mismatch {
        name: String,
        expected: String,
        actual: String,
    },

    #[error("{name}: not in cache and --offline mode (expected at {})", cache_path.display())]
    OfflineMissing { name: String, cache_path: PathBuf },
}

/// Resolved per-dep fetch parameters: where to download from, where to
/// cache the bytes, and what sha256 to verify against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchPlan {
    /// Display name for progress / errors, e.g. "assay-engine 0.4.1".
    pub display_name: String,
    /// Resolved URL (default release convention or the manifest's `source`).
    pub url: String,
    /// Where the artifact lands in the cache.
    pub cache_path: PathBuf,
    /// Expected sha256 hex (lowercase).
    pub expected_sha256: String,
}

impl FetchPlan {
    /// Build a plan for an extension binary on `arch`.
    pub fn for_extension(
        ext: &Extension,
        arch: &str,
        cache_dir: &Path,
    ) -> Result<Self, FetchError> {
        let display_name = format!("{} {}", ext.name, ext.version);
        let expected_sha256 = ext
            .sha256
            .get(arch)
            .ok_or_else(|| FetchError::NoArchHash {
                name: display_name.clone(),
                arch: arch.to_string(),
                available: ext.sha256.keys().cloned().collect(),
            })?
            .clone();

        let filename = format!("{}-{}-{}.tar.gz", ext.name, ext.version, arch);
        let url = ext
            .source
            .clone()
            .unwrap_or_else(|| default_extension_url(&ext.name, &ext.version, arch));
        let cache_path = cache_dir.join(&filename);
        Ok(FetchPlan {
            display_name,
            url,
            cache_path,
            expected_sha256,
        })
    }

    /// Build a plan for a Lua library tarball (arch-neutral).
    pub fn for_lib(lib: &Lib, cache_dir: &Path) -> Self {
        let display_name = format!("{} {}", lib.name, lib.version);
        let filename = format!("assay-lib-{}-{}.tar.gz", lib.name, lib.version);
        let url = lib
            .source
            .clone()
            .unwrap_or_else(|| default_lib_url(&lib.name, &lib.version));
        let cache_path = cache_dir.join(&filename);
        FetchPlan {
            display_name,
            url,
            cache_path,
            expected_sha256: lib.sha256.clone(),
        }
    }
}

fn default_extension_url(name: &str, version: &str, arch: &str) -> String {
    // Convention: `<RELEASE_BASE>/v<version>/<name>-<version>-<arch>.tar.gz`.
    // Subject to refinement in plan 21 phase 5 (release pipeline).
    format!("{RELEASE_BASE}/v{version}/{name}-{version}-{arch}.tar.gz")
}

fn default_lib_url(name: &str, version: &str) -> String {
    format!("{RELEASE_BASE}/assay-lib-{name}-v{version}/assay-lib-{name}-{version}.tar.gz")
}

/// Ensure `plan.cache_path` exists and matches `plan.expected_sha256`.
pub async fn fetch(
    plan: &FetchPlan,
    client: &reqwest::Client,
    offline: bool,
) -> Result<(), FetchError> {
    // 1+2: cache probe.
    if afs::try_exists(&plan.cache_path).await.unwrap_or(false) {
        let bytes = afs::read(&plan.cache_path).await.map_err(io_err(plan))?;
        let actual = sha256_hex(&bytes);
        if actual == plan.expected_sha256 {
            return Ok(());
        }
        // Bad cache entry: drop it. If a parallel fetch beats us to the
        // delete, that's fine — `remove_file` racing with itself is benign.
        let _ = afs::remove_file(&plan.cache_path).await;
    }

    if offline {
        return Err(FetchError::OfflineMissing {
            name: plan.display_name.clone(),
            cache_path: plan.cache_path.clone(),
        });
    }

    // 4: download → tmp → verify → rename.
    let parent = plan
        .cache_path
        .parent()
        .expect("cache_path always has a parent (it's <cache-dir>/<filename>)");
    afs::create_dir_all(parent).await.map_err(io_err(plan))?;

    let resp = client
        .get(&plan.url)
        .send()
        .await
        .map_err(|e| FetchError::Http {
            name: plan.display_name.clone(),
            url: plan.url.clone(),
            source: e,
        })?;
    if !resp.status().is_success() {
        return Err(FetchError::Status {
            name: plan.display_name.clone(),
            url: plan.url.clone(),
            status: resp.status().as_u16(),
        });
    }
    let bytes = resp.bytes().await.map_err(|e| FetchError::Http {
        name: plan.display_name.clone(),
        url: plan.url.clone(),
        source: e,
    })?;

    let actual = sha256_hex(&bytes);
    if actual != plan.expected_sha256 {
        return Err(FetchError::Sha256Mismatch {
            name: plan.display_name.clone(),
            expected: plan.expected_sha256.clone(),
            actual,
        });
    }

    let mut tmp_path = plan.cache_path.clone();
    let mut tmp_name = plan
        .cache_path
        .file_name()
        .expect("cache_path has a filename")
        .to_os_string();
    tmp_name.push(".tmp");
    tmp_path.set_file_name(tmp_name);

    let mut tmp = afs::File::create(&tmp_path).await.map_err(io_err(plan))?;
    tmp.write_all(&bytes).await.map_err(io_err(plan))?;
    tmp.flush().await.map_err(io_err(plan))?;
    drop(tmp);
    afs::rename(&tmp_path, &plan.cache_path)
        .await
        .map_err(io_err(plan))?;

    Ok(())
}

fn io_err(plan: &FetchPlan) -> impl Fn(io::Error) -> FetchError + '_ {
    let name = plan.display_name.clone();
    move |source| FetchError::Io {
        name: name.clone(),
        source,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    HEXLOWER.encode(&hasher.finalize())
}
