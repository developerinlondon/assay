mod common;

use common::create_vm;

#[tokio::test]
async fn test_apt_table_registered() {
    let vm = create_vm();
    let names: Vec<String> = vm
        .load(r#"
            local out = {}
            for k, _ in pairs(apt) do out[#out+1] = k end
            table.sort(out)
            return out
        "#)
        .eval_async()
        .await
        .unwrap();
    let want = vec![
        "add_source", "install", "list_installed", "list_upgradable",
        "query", "remove", "update",
    ];
    for w in want {
        assert!(names.iter().any(|n| n == w), "missing apt.{w}; have {names:?}");
    }
}

#[tokio::test]
async fn test_apt_parse_dpkg_query_output() {
    let vm = create_vm();
    let result: mlua::Table = vm
        .load(r#"
            local sample = [[
cloudflared	2024.10.1	install ok installed
tailscale	1.78.1	install ok installed
ghost	1.0	deinstall ok config-files
]]
            return apt._parse_dpkg_lines(sample)
        "#)
        .eval_async()
        .await
        .unwrap();
    let cloudflared: mlua::Table = result.get("cloudflared").unwrap();
    let v: String = cloudflared.get("version").unwrap();
    assert_eq!(v, "2024.10.1");
    let installed: bool = cloudflared.get("installed").unwrap();
    assert!(installed);

    // ghost is deinstalled — treated as not installed.
    let ghost: Option<mlua::Table> = result.get("ghost").ok();
    if let Some(g) = ghost {
        let inst: bool = g.get("installed").unwrap_or(true);
        assert!(!inst);
    }
}

#[tokio::test]
async fn test_apt_parse_upgradable_output() {
    let vm = create_vm();
    let result: mlua::Table = vm
        .load(r#"
            local sample = [[
Listing...
cloudflared/bookworm 2024.10.1 amd64 [upgradable from: 2024.9.0]
tailscale/bookworm 1.78.1 amd64 [upgradable from: 1.76.0]
N: There is 1 additional version. Please use the '-a' switch to see it
]]
            return apt._parse_upgradable_lines(sample)
        "#)
        .eval_async()
        .await
        .unwrap();
    let cf: mlua::Table = result.get(1).unwrap();
    let name: String = cf.get("name").unwrap();
    let cur: String = cf.get("current").unwrap();
    let cand: String = cf.get("candidate").unwrap();
    assert_eq!(name, "cloudflared");
    assert_eq!(cur, "2024.9.0");
    assert_eq!(cand, "2024.10.1");
}

#[tokio::test]
async fn test_apt_add_source_writes_files_and_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let sources_dir = dir.path().join("sources.list.d");
    let keyrings_dir = dir.path().join("keyrings");
    std::fs::create_dir_all(&sources_dir).unwrap();
    std::fs::create_dir_all(&keyrings_dir).unwrap();

    let fake_key = dir.path().join("fake-key.gpg");
    std::fs::write(&fake_key, b"-----BEGIN PGP PUBLIC KEY BLOCK-----\nfake\n").unwrap();

    let vm = create_vm();
    vm.globals().set("sources_dir", sources_dir.to_str().unwrap().to_string()).unwrap();
    vm.globals().set("keyrings_dir", keyrings_dir.to_str().unwrap().to_string()).unwrap();
    vm.globals().set("key_path", fake_key.to_str().unwrap().to_string()).unwrap();

    let res1: mlua::Table = vm
        .load(r#"
            return apt.add_source({
                id           = "test-vendor",
                source_list  = "deb https://example.com/repo bookworm main",
                key_path     = key_path,
                _sources_dir = sources_dir,
                _keyrings_dir = keyrings_dir,
            })
        "#)
        .eval_async()
        .await
        .unwrap();

    let changed1: bool = res1.get("changed").unwrap();
    assert!(changed1, "first add should report changed=true");
    let list_path = sources_dir.join("test-vendor.list");
    let key_dst = keyrings_dir.join("test-vendor.gpg");
    assert!(list_path.exists());
    assert!(key_dst.exists());

    let content = std::fs::read_to_string(&list_path).unwrap();
    assert!(content.contains("https://example.com/repo bookworm main"));

    // Idempotency: second call should report changed=false.
    let res2: mlua::Table = vm
        .load(r#"
            return apt.add_source({
                id           = "test-vendor",
                source_list  = "deb https://example.com/repo bookworm main",
                key_path     = key_path,
                _sources_dir = sources_dir,
                _keyrings_dir = keyrings_dir,
            })
        "#)
        .eval_async()
        .await
        .unwrap();
    let changed2: bool = res2.get("changed").unwrap();
    assert!(!changed2, "second add with identical inputs should report changed=false");
}
