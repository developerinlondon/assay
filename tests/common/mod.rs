use std::time::Duration;

pub fn create_vm() -> mlua::Lua {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    assay::lua::create_vm(client).unwrap()
}

pub async fn run_lua(script: &str) -> Result<(), mlua::Error> {
    let vm = create_vm();
    vm.load(script).exec_async().await
}

#[allow(dead_code)]
pub async fn eval_lua<T: mlua::FromLua>(script: &str) -> T {
    let vm = create_vm();
    vm.load(script).eval_async::<T>().await.unwrap()
}
