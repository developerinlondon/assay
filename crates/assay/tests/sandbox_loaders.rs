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
    let vm = make_vm_with_env("os.execute,os.exit");
    let script = r#"
        assert.eq(os.execute, nil)
        assert.eq(os.exit, nil)
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
