use std::time::Duration;

#[allow(dead_code)]
pub fn create_vm() -> mlua::Lua {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    assay::lua::create_vm(client).unwrap()
}

#[allow(dead_code)]
pub async fn run_lua(script: &str) -> Result<(), mlua::Error> {
    let vm = create_vm();
    let script = assay::lua::async_bridge::strip_shebang(script);
    vm.load(script).exec_async().await
}

#[allow(dead_code)]
pub async fn eval_lua<T: mlua::FromLua>(script: &str) -> T {
    let vm = create_vm();
    let script = assay::lua::async_bridge::strip_shebang(script);
    vm.load(script).eval_async::<T>().await.unwrap()
}

#[allow(dead_code)]
pub async fn run_lua_local(script: &str) -> Result<(), mlua::Error> {
    let vm = create_vm();
    let script = assay::lua::async_bridge::strip_shebang(script);
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async { vm.load(script).exec_async().await })
        .await
}

#[allow(dead_code)]
pub async fn eval_lua_local<T: mlua::FromLua>(script: &str) -> T {
    let vm = create_vm();
    let script = assay::lua::async_bridge::strip_shebang(script);
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async { vm.load(script).eval_async::<T>().await.unwrap() })
        .await
}
