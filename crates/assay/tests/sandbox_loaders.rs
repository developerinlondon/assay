// Sandbox tweaks shipped in 0.14.2: source-level loaders are usable by
// default; an opt-in ASSAY_BLOCK_GLOBALS env var lets ops nil out
// arbitrary globals (incl. dotted stdlib paths). string.dump stays
// blocked unconditionally because it produces native bytecode that
// defeats the runtime's memory/CPU caps.

// The ASSAY_BLOCK_GLOBALS env var is read at VM construction time
// (process-wide). cargo's default test harness runs tests on threads
// in the same process, so any test that builds a VM races against any
// test that mutates that env var. ENV_LOCK serializes ALL VM creation
// in this file. Crucially, the lock is held ONLY across the synchronous
// `create_vm` call — never across an `await` — so we don't trip
// `clippy::await_holding_lock`.

use std::sync::Mutex;
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn make_vm() -> mlua::Lua {
    let _g = ENV_LOCK.lock().unwrap();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    assay::lua::create_vm(client).unwrap()
    // _g dropped here, before any caller awaits.
}

fn make_vm_with_env(blocks: &str) -> mlua::Lua {
    let _g = ENV_LOCK.lock().unwrap();
    // SAFETY: ENV_LOCK serialises every test in this file that mutates
    // ASSAY_BLOCK_GLOBALS or constructs a VM that reads it.
    unsafe {
        std::env::set_var(assay::lua::BLOCK_GLOBALS_ENV, blocks);
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    let vm = assay::lua::create_vm(client).unwrap();
    unsafe {
        std::env::remove_var(assay::lua::BLOCK_GLOBALS_ENV);
    }
    vm
    // _g dropped here.
}

async fn run(script: &str, vm: mlua::Lua) {
    let bridged = assay::lua::async_bridge::strip_shebang(script);
    vm.load(bridged).exec_async().await.unwrap();
}

#[tokio::test]
async fn load_loadfile_dofile_are_callable_by_default() {
    let vm = make_vm();
    let script = r#"
        assert.eq(type(load), "function")
        assert.eq(type(loadfile), "function")
        assert.eq(type(dofile), "function")

        -- load() with a chunk string returns a function, not nil.
        local fn = load("return 1 + 2")
        assert.eq(type(fn), "function")
        assert.eq(fn(), 3)
    "#;
    run(script, vm).await;
}

#[tokio::test]
async fn string_dump_stays_blocked_for_safety() {
    let vm = make_vm();
    let script = r#"
        -- string.dump produces Lua bytecode and is the documented
        -- bytecode-escape hatch — must be unavailable.
        assert.eq(string.dump, nil)
    "#;
    run(script, vm).await;
}

#[tokio::test]
async fn assay_block_globals_nils_top_level_names() {
    let vm = make_vm_with_env("dofile,loadfile");
    let script = r#"
        assert.eq(dofile, nil)
        assert.eq(loadfile, nil)
        assert.eq(type(load), "function")
    "#;
    run(script, vm).await;
}

#[tokio::test]
async fn assay_block_globals_nils_dotted_paths() {
    // `string` is a table assay does not shadow, so `_G.string` and
    // `package.loaded.string` are one table and the assertion is about the
    // real library. Asserting `os.execute == nil` on `_G.os` would pass
    // whatever the block list did, because assay's `os` never had it.
    let vm = make_vm_with_env("string.rep,os.execute,os.exit");
    let script = r#"
        assert.eq(string.rep, nil)
        assert.eq(type(string.sub), "function")

        local os_lib = require("os")
        assert.eq(os_lib.execute, nil)
        assert.eq(os_lib.exit, nil)
        assert.eq(type(os_lib.time), "function")
        assert.eq(type(os.time), "function")
    "#;
    run(script, vm).await;
}

#[tokio::test]
async fn assay_block_globals_silently_skips_typos() {
    // Unknown table prefixes (no `bogus` table in globals) and pure
    // whitespace entries must not error VM creation — that would turn
    // a typo into a CrashLoopBackOff.
    let vm = make_vm_with_env("bogus.path, , dofile");
    let script = r#"
        assert.eq(dofile, nil)
        assert.eq(bogus, nil)
    "#;
    run(script, vm).await;
}

// Until 0.20.14 the env list ran before `register_all`, so it could only
// reach names Lua itself had installed: naming `fs` cleared nothing, because
// the builtins were registered afterwards and put it back.
#[tokio::test]
async fn assay_block_globals_reaches_assay_s_own_globals() {
    let vm = make_vm_with_env("fs,db,dns,ws");
    let script = r#"
        assert.eq(fs, nil)
        assert.eq(db, nil)
        assert.eq(dns, nil)
        assert.eq(ws, nil)
        assert.eq(type(http), "table")
    "#;
    run(script, vm).await;
}

#[tokio::test]
async fn assay_block_globals_mixes_stdlib_and_builtin_names() {
    let vm = make_vm_with_env("os.execute,fs");
    // Asserted against the table `require` returns, not `_G.os`: assay puts
    // its own `os` there and it never had `execute`, so checking the global
    // would pass without the block list doing anything at all.
    //
    // This one holds by the early pass rather than the late one: `sandbox`
    // runs before `os_info` replaces `_G.os`, so at that moment the global
    // IS Lua's real table, the same one `package.loaded.os` holds. The late
    // pass is what a policy needs, and the policy suite proves that path.
    let script = r#"
        local os_lib = require("os")
        assert.eq(os_lib.execute, nil)
        assert.eq(type(os_lib.time), "function")
        assert.eq(fs, nil)
    "#;
    run(script, vm).await;
}

#[tokio::test]
async fn assay_block_globals_clears_the_require_cache_for_a_bare_name() {
    let vm = make_vm_with_env("io");
    let script = r#"
        assert.eq(io, nil)
        assert.eq(package.loaded["io"], nil)
        assert.eq(pcall(require, "io"), false)
    "#;
    run(script, vm).await;
}

// Blocking a name used to make a read-only VM *more* capable. The env list
// ran before registration, so naming `io` deleted `_G.io`; the read-only gate
// then skipped it, because it skips a table that is not on `_G`; and the real,
// ungated table stayed in `package.loaded` for `require` to hand back. Both
// halves are covered now: the gates run first, and the block clears both roots.
#[tokio::test]
async fn blocking_a_table_never_leaves_an_ungated_one_behind() {
    let vm = {
        let _g = ENV_LOCK.lock().unwrap();
        unsafe {
            std::env::set_var(assay::lua::BLOCK_GLOBALS_ENV, "io,os.execute,fs");
            std::env::set_var(assay::lua::READONLY_ENV, "1");
        }
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        let vm = assay::lua::create_vm(client).unwrap();
        unsafe {
            std::env::remove_var(assay::lua::BLOCK_GLOBALS_ENV);
            std::env::remove_var(assay::lua::READONLY_ENV);
        }
        vm
    };
    let script = r#"
        assert.eq(io, nil)
        assert.eq(fs, nil)
        assert.eq(package.loaded["io"], nil)
        assert.eq(pcall(require, "io"), false)

        -- and nothing ungated survives behind require for the rest of it
        local os_lib = require("os")
        assert.eq(os_lib.execute, nil)
        local ok, err = pcall(os_lib.remove, "/tmp/assay-block-probe")
        assert.eq(ok, false)
        assert.eq(tostring(err):find("readonly:") ~= nil, true)
    "#;
    run(script, vm).await;
}
