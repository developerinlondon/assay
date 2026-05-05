//! Extract `.tar.gz` artifacts from the cache into the destination
//! filesystem paths.
//!
//! Two install shapes:
//! - **Extension binary** — pull one named member out of the tarball and
//!   atomic-rename it into `<bin_dir>/<bin_name>` with mode 0755.
//! - **Lib tree** — extract the whole tarball into `<lib_dir>/<lib_name>/`,
//!   replacing any existing tree at that path.
//!
//! ### Tarball layout assumptions
//!
//! - Extensions: tarball contains the binary as a top-level entry whose
//!   filename matches `bin_name` (e.g. an archive containing `assay-engine`).
//! - Libs: tarball contains the lib's tree at the root (i.e. extracting the
//!   archive into a directory yields the lib's contents directly, *not* a
//!   `<name>/` wrapping directory).
//!
//! The release pipeline (plan 21 phase 5) finalises both conventions.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use tar::Archive;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExtractError {
    #[error("{name}: I/O error: {source}")]
    Io {
        name: String,
        #[source]
        source: std::io::Error,
    },

    #[error("{name}: archive `{archive}` contains no top-level entry named `{member}`")]
    BinaryMemberNotFound {
        name: String,
        archive: PathBuf,
        member: String,
    },
}

/// Install one extension binary from `archive_path` into
/// `<bin_dir>/<bin_name>` (mode 0755 on unix), replacing any existing file.
pub fn install_extension_binary(
    archive_path: &Path,
    bin_dir: &Path,
    bin_name: &str,
) -> Result<PathBuf, ExtractError> {
    let name = bin_name.to_string();
    fs::create_dir_all(bin_dir).map_err(io(&name))?;

    let bytes = read_member(archive_path, bin_name)
        .map_err(io(&name))?
        .ok_or_else(|| ExtractError::BinaryMemberNotFound {
            name: name.clone(),
            archive: archive_path.to_path_buf(),
            member: bin_name.to_string(),
        })?;

    let final_path = bin_dir.join(bin_name);
    let tmp_path = bin_dir.join(format!(".{bin_name}.tmp"));
    // Best-effort cleanup of leftover tmp from a prior crashed install.
    let _ = fs::remove_file(&tmp_path);

    fs::write(&tmp_path, &bytes).map_err(io(&name))?;
    set_executable_mode(&tmp_path).map_err(io(&name))?;
    fs::rename(&tmp_path, &final_path).map_err(io(&name))?;

    Ok(final_path)
}

/// Install a Lua library tree from `archive_path` into
/// `<lib_dir>/<lib_name>/`, replacing any existing tree at that path.
///
/// Strategy: extract into a sibling staging directory `.<name>.new`,
/// then swap (`rm -rf <name>` if present, `mv <name>.new <name>`). The
/// swap window is small but not strictly atomic for non-empty target
/// directories — POSIX `rename` only clobbers empty directories.
pub fn install_lib_tree(
    archive_path: &Path,
    lib_dir: &Path,
    lib_name: &str,
) -> Result<PathBuf, ExtractError> {
    let name = lib_name.to_string();
    fs::create_dir_all(lib_dir).map_err(io(&name))?;

    let target = lib_dir.join(lib_name);
    let staging = lib_dir.join(format!(".{lib_name}.new"));

    // Drop any leftover staging from a prior crashed install.
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(io(&name))?;
    }
    fs::create_dir_all(&staging).map_err(io(&name))?;

    let f = fs::File::open(archive_path).map_err(io(&name))?;
    let mut archive = Archive::new(GzDecoder::new(f));
    archive.unpack(&staging).map_err(io(&name))?;

    if target.exists() {
        fs::remove_dir_all(&target).map_err(io(&name))?;
    }
    fs::rename(&staging, &target).map_err(io(&name))?;

    Ok(target)
}

fn read_member(archive_path: &Path, member: &str) -> std::io::Result<Option<Vec<u8>>> {
    let target_basename = Path::new(member);
    let f = fs::File::open(archive_path)?;
    let mut archive = Archive::new(GzDecoder::new(f));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        if path.file_name() == Some(target_basename.as_os_str()) {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            return Ok(Some(buf));
        }
    }
    Ok(None)
}

fn io(name: &str) -> impl Fn(std::io::Error) -> ExtractError + '_ {
    let owned = name.to_string();
    move |source| ExtractError::Io {
        name: owned.clone(),
        source,
    }
}

#[cfg(unix)]
fn set_executable_mode(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn set_executable_mode(_path: &Path) -> std::io::Result<()> {
    Ok(())
}
