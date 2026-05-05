//! Confirms that registering a FileSource on the Lua VM redirects
//! fs.read / fs.read_bytes through it instead of hitting disk.

use assay::lua::file_source::{FileSource, set_file_source};
use std::sync::Arc;

struct Fixture(Vec<(String, Vec<u8>)>);
impl FileSource for Fixture {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.0
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, b)| b.clone())
    }
}

#[test]
fn fs_read_consults_registered_source() {
    let lua = mlua::Lua::new();
    // Use the same registration sequence as create_vm but skip the
    // pieces irrelevant to fs.read (require loader, sandbox, etc.).
    assay::lua::builtins::core::register_fs(&lua).unwrap();

    let src: Arc<dyn FileSource> = Arc::new(Fixture(vec![(
        "virtual/hello.txt".into(),
        b"hi from embed".to_vec(),
    )]));
    set_file_source(&lua, src);

    let result: String = lua
        .load(r#"return fs.read("virtual/hello.txt")"#)
        .eval()
        .unwrap();
    assert_eq!(result, "hi from embed");
}

#[test]
fn fs_read_errors_on_source_miss_when_source_registered() {
    let lua = mlua::Lua::new();
    assay::lua::builtins::core::register_fs(&lua).unwrap();
    let src: Arc<dyn FileSource> = Arc::new(Fixture(vec![]));
    set_file_source(&lua, src);

    let err = lua
        .load(r#"return fs.read("not-in-source")"#)
        .eval::<String>()
        .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("not-in-source"),
        "error should mention the missing path: {msg}"
    );
}

#[test]
fn fs_read_falls_back_to_disk_when_no_source_registered() {
    let lua = mlua::Lua::new();
    assay::lua::builtins::core::register_fs(&lua).unwrap();

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"on disk").unwrap();
    let path = tmp.path().to_str().unwrap();

    let result: String = lua
        .load(format!(r#"return fs.read("{path}")"#))
        .eval()
        .unwrap();
    assert_eq!(result, "on disk");
}
