//! Pluggable file source for Lua's `fs.read` / `fs.read_bytes`.
//!
//! Consumers (e.g. embedded-app binaries) implement `FileSource` to
//! redirect the runtime's filesystem reads through their own backing
//! store — typically a `rust-embed` virtual FS plus optional disk
//! overlays for operator config.
//!
//! When no FileSource is registered, `fs.read` falls back to direct
//! disk reads (preserving the standalone `assay run script.lua`
//! behaviour).

use std::sync::Arc;

/// Anything that can resolve a path string to bytes. Implementations
/// MUST be cheap to clone (we hold `Arc<dyn FileSource>`) and `Send +
/// Sync` so they can live in mlua's app-data across async tasks.
pub trait FileSource: Send + Sync {
    /// Return the bytes at `path`, or `None` if the path is unknown.
    /// Errors during read are surfaced as `None`; this matches Lua's
    /// existing "missing file" behaviour where `fs.read` raises a
    /// runtime error.
    fn read(&self, path: &str) -> Option<Vec<u8>>;
}

/// Default FileSource that reads directly from disk. Available for
/// callers who want to register a source explicitly but keep
/// disk-backed semantics.
#[allow(dead_code)]
pub struct DiskFileSource;

impl FileSource for DiskFileSource {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(path).ok()
    }
}

/// Type-erased handle stored in mlua's app-data so the Lua VM can
/// retrieve the registered source from inside `fs.read` closures.
pub(crate) type FileSourceHandle = Arc<dyn FileSource>;

/// Register a `FileSource` with the given Lua state. Subsequent calls
/// to `fs.read` / `fs.read_bytes` consult this source instead of
/// reading from disk directly.
#[allow(dead_code)]
pub fn set_file_source(lua: &mlua::Lua, source: Arc<dyn FileSource>) {
    lua.set_app_data::<FileSourceHandle>(source);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct InMemSource(Vec<(String, Vec<u8>)>);
    impl FileSource for InMemSource {
        fn read(&self, path: &str) -> Option<Vec<u8>> {
            self.0
                .iter()
                .find(|(p, _)| p == path)
                .map(|(_, b)| b.clone())
        }
    }

    #[test]
    fn disk_source_reads_real_file() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"hello").unwrap();
        let src = DiskFileSource;
        let bytes = src.read(tmp.path().to_str().unwrap()).unwrap();
        assert_eq!(bytes, b"hello");
    }

    #[test]
    fn disk_source_returns_none_for_missing() {
        let src = DiskFileSource;
        assert!(src.read("/definitely/does/not/exist").is_none());
    }

    #[test]
    fn in_mem_source_returns_registered_path() {
        let src = InMemSource(vec![("hi.txt".into(), b"hello".to_vec())]);
        assert_eq!(src.read("hi.txt").unwrap(), b"hello");
        assert!(src.read("missing").is_none());
    }

    #[test]
    fn set_file_source_stores_handle_in_app_data() {
        let lua = mlua::Lua::new();
        let src: Arc<dyn FileSource> = Arc::new(DiskFileSource);
        set_file_source(&lua, src);
        assert!(lua.app_data_ref::<FileSourceHandle>().is_some());
    }
}
