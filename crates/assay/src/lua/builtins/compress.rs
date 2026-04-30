//! Compression / archive builtins: gunzip, unxz, unzstd, untar.
//!
//! Each function takes a Lua string (treated as raw bytes) and returns the
//! decompressed bytes as a Lua string. Lua strings in mlua are byte buffers,
//! so binary data round-trips cleanly.

use std::io::Read;

pub fn register_compress(lua: &mlua::Lua) -> mlua::Result<()> {
    let t = lua.create_table()?;
    t.set("gunzip", lua.create_function(gunzip)?)?;
    t.set("unxz", lua.create_function(unxz)?)?;
    t.set("unzstd", lua.create_function(unzstd)?)?;
    t.set("untar", lua.create_function(untar)?)?;
    lua.globals().set("compress", t)?;
    Ok(())
}

fn gunzip(lua: &mlua::Lua, input: mlua::String) -> mlua::Result<mlua::String> {
    let bytes = input.as_bytes();
    let mut decoder = flate2::read::GzDecoder::new(&bytes[..]);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|e| mlua::Error::runtime(format!("compress.gunzip: {e}")))?;
    lua.create_string(&out)
}

fn unxz(lua: &mlua::Lua, input: mlua::String) -> mlua::Result<mlua::String> {
    let bytes = input.as_bytes();
    let mut decoder = xz2::read::XzDecoder::new(&bytes[..]);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|e| mlua::Error::runtime(format!("compress.unxz: {e}")))?;
    lua.create_string(&out)
}

fn unzstd(lua: &mlua::Lua, input: mlua::String) -> mlua::Result<mlua::String> {
    let bytes = input.as_bytes();
    let out = zstd::stream::decode_all(&bytes[..])
        .map_err(|e| mlua::Error::runtime(format!("compress.unzstd: {e}")))?;
    lua.create_string(&out)
}

/// compress.untar(archive_path, dest_path, opts)
///
/// Extracts a single named member from a tar archive (optionally gzip/xz/zstd
/// compressed) to dest_path. Returns the number of bytes written.
///
/// Opts:
///   member      = "<path-inside-archive>"  (REQUIRED)
///   compression = "gz" | "xz" | "zstd" | "none"  (auto-detected from archive_path
///                                                 extension if omitted)
fn untar(_: &mlua::Lua, args: mlua::MultiValue) -> mlua::Result<i64> {
    use std::io::Read;

    let mut args_iter = args.into_iter();
    let archive_path: String = match args_iter.next() {
        Some(mlua::Value::String(s)) => s.to_str()?.to_string(),
        _ => {
            return Err(mlua::Error::runtime(
                "compress.untar: first arg must be archive path string",
            ));
        }
    };
    let dest_path: String = match args_iter.next() {
        Some(mlua::Value::String(s)) => s.to_str()?.to_string(),
        _ => {
            return Err(mlua::Error::runtime(
                "compress.untar: second arg must be dest path string",
            ));
        }
    };
    let opts: mlua::Table = match args_iter.next() {
        Some(mlua::Value::Table(t)) => t,
        _ => {
            return Err(mlua::Error::runtime(
                "compress.untar: third arg must be opts table with 'member'",
            ));
        }
    };
    let member: String = opts
        .get::<String>("member")
        .map_err(|_| mlua::Error::runtime("compress.untar: opts.member is required"))?;
    let compression: String = opts
        .get::<Option<String>>("compression")?
        .map(|s| s.to_lowercase())
        .unwrap_or_else(|| detect_compression(&archive_path));

    let file = std::fs::File::open(&archive_path)
        .map_err(|e| mlua::Error::runtime(format!("compress.untar: open {archive_path:?}: {e}")))?;

    // Build a Read trait object based on compression
    let reader: Box<dyn Read> = match compression.as_str() {
        "gz" | "gzip" => Box::new(flate2::read::GzDecoder::new(file)),
        "xz" => Box::new(xz2::read::XzDecoder::new(file)),
        "zstd" | "zst" => Box::new(
            zstd::stream::read::Decoder::new(file)
                .map_err(|e| mlua::Error::runtime(format!("compress.untar: zstd decoder: {e}")))?,
        ),
        "none" | "tar" => Box::new(file),
        other => {
            return Err(mlua::Error::runtime(format!(
                "compress.untar: unsupported compression {other:?} (gz, xz, zstd, none)"
            )));
        }
    };

    let mut archive = tar::Archive::new(reader);
    for entry in archive
        .entries()
        .map_err(|e| mlua::Error::runtime(format!("compress.untar: read entries: {e}")))?
    {
        let mut entry =
            entry.map_err(|e| mlua::Error::runtime(format!("compress.untar: entry: {e}")))?;
        let path_in_tar = entry
            .path()
            .map_err(|e| mlua::Error::runtime(format!("compress.untar: entry path: {e}")))?;
        // Safety: dest_path is supplied by the caller; the in-tar path
        // is used only for member-name matching and is never written to
        // disk. If this is ever extended to extract a tree, sanitize
        // entry paths to prevent path-traversal (CVE-2007-4559).
        if path_in_tar.to_string_lossy() == member {
            // Ensure dest parent exists
            if let Some(parent) = std::path::Path::new(&dest_path).parent()
                && !parent.as_os_str().is_empty()
            {
                std::fs::create_dir_all(parent).map_err(|e| {
                    mlua::Error::runtime(format!("compress.untar: mkdir parent: {e}"))
                })?;
            }
            // Stream member to dest via temp file for atomicity
            let tmp = format!("{dest_path}.tmp.{}", std::process::id());
            let mut out = std::fs::File::create(&tmp).map_err(|e| {
                mlua::Error::runtime(format!("compress.untar: create temp {tmp:?}: {e}"))
            })?;
            let n = std::io::copy(&mut entry, &mut out).map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                mlua::Error::runtime(format!("compress.untar: write: {e}"))
            })?;
            drop(out);
            std::fs::rename(&tmp, &dest_path).map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                mlua::Error::runtime(format!("compress.untar: rename: {e}"))
            })?;
            return Ok(n as i64);
        }
    }
    Err(mlua::Error::runtime(format!(
        "compress.untar: member {member:?} not found in {archive_path:?}"
    )))
}

fn detect_compression(path: &str) -> String {
    let lower = path.to_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        "gz".into()
    } else if lower.ends_with(".tar.xz") || lower.ends_with(".txz") {
        "xz".into()
    } else if lower.ends_with(".tar.zst") || lower.ends_with(".tzst") {
        "zstd".into()
    } else {
        "none".into()
    }
}
