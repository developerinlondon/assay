mod common;

use common::run_lua;
use tempfile::TempDir;

#[tokio::test]
async fn test_tar_builtin_available() {
    let script = r#"
        assert.not_nil(tar)
        assert.not_nil(tar.create)
        assert.not_nil(tar.extract)
        assert.not_nil(tar.list)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn test_tar_create_and_extract() {
    let dir = TempDir::new().unwrap();
    let output = dir.path().join("test.tar.gz");

    let script = format!(
        r#"
        local ok = tar.create("{}", {{
            ["hello.txt"] = "Hello, World!",
            ["sub/config.toml"] = "[server]\nport = 8080\n",
        }}, {{gzip = true}})
        assert.eq(ok, true)
        "#,
        output.display()
    );
    run_lua(&script).await.unwrap();

    assert!(output.exists());

    // Extract and verify
    let extract_dir = dir.path().join("extracted");
    let script = format!(
        r#"
        local ok = tar.extract("{}", "{}")
        assert.eq(ok, true)
        "#,
        output.display(),
        extract_dir.display()
    );
    run_lua(&script).await.unwrap();

    let hello = std::fs::read_to_string(extract_dir.join("hello.txt")).unwrap();
    assert_eq!(hello, "Hello, World!");

    let config = std::fs::read_to_string(extract_dir.join("sub/config.toml")).unwrap();
    assert_eq!(config, "[server]\nport = 8080\n");
}

#[tokio::test]
async fn test_tar_list() {
    let dir = TempDir::new().unwrap();
    let output = dir.path().join("bundle.tar.gz");

    let script = format!(
        r#"
        tar.create("{}", {{
            ["a.txt"] = "a",
            ["b.txt"] = "b",
            ["c/d.txt"] = "d",
        }}, {{gzip = true}})
        local paths = tar.list("{}")
        assert.eq(#paths, 3)
        "#,
        output.display(),
        output.display()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_tar_create_uncompressed() {
    let dir = TempDir::new().unwrap();
    let output = dir.path().join("plain.tar");

    let script = format!(
        r#"
        local ok = tar.create("{}", {{
            ["file.txt"] = "plain tar content",
        }}, {{gzip = false}})
        assert.eq(ok, true)
        "#,
        output.display()
    );
    run_lua(&script).await.unwrap();

    assert!(output.exists());
}
