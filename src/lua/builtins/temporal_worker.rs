/// Temporal worker runtime — bridges Lua activities and workflows to Temporal via CoreWorker.
///
/// Uses the low-level `temporalio-sdk-core` Worker API directly (Path A) for full control
/// over activity/workflow dispatch to Lua functions. This avoids the proc-macro-based
/// registration in the high-level SDK and gives us direct access to task polling and
/// completion.
///
/// ## Architecture
///
/// ```text
/// temporal.worker(opts)
///   │
///   ├── Activity Loop (tokio task)
///   │     poll_activity_task() → lookup Lua fn by type → call → complete_activity_task()
///   │
///   └── Workflow Loop (tokio LocalSet)
///         poll_workflow_activation() → dispatch jobs to Lua coroutine → complete_workflow_activation()
/// ```
#[cfg(feature = "temporal")]
pub fn register_temporal_worker(lua: &mlua::Lua) -> mlua::Result<()> {
    use mlua::{Function, RegistryKey, Table, Value};
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use temporalio_client::{Connection, ConnectionOptions};
    use temporalio_common::{
        protos::{
            coresdk::{
                ActivityTaskCompletion,
                activity_result::ActivityExecutionResult,
                activity_task,
            },
            temporal::api::{
                common::v1::Payload,
                failure::v1::Failure,
            },
        },
        worker::WorkerTaskTypes,
    };
    use temporalio_sdk_core::{
        CoreRuntime, PollError, PollerBehavior, RuntimeOptions, Worker as CoreWorker,
        WorkerConfig, WorkerVersioningStrategy, init_worker,
    };
    use temporalio_common::worker::WorkerDeploymentOptions;

    /// Encode a JSON string as a Temporal Payload with json/plain encoding.
    fn json_payload(json_str: &str) -> Payload {
        Payload {
            metadata: std::collections::HashMap::from([(
                "encoding".to_string(),
                b"json/plain".to_vec(),
            )]),
            data: json_str.as_bytes().to_vec(),
            ..Default::default()
        }
    }

    /// Decode a Temporal Payload back to a JSON string.
    fn payload_to_json(payload: &Payload) -> Result<String, String> {
        std::str::from_utf8(&payload.data)
            .map(|s| s.to_string())
            .map_err(|e| format!("invalid UTF-8 in payload: {e}"))
    }

    let temporal: Table = lua.globals().get("temporal")?;

    // temporal.worker({ url, namespace?, task_queue, activities?, workflows? })
    let worker_fn = lua.create_async_function(|lua, opts: Table| async move {
        let url: String = opts.get("url")?;
        let namespace: String = opts
            .get::<Option<String>>("namespace")?
            .unwrap_or_else(|| "default".to_string());
        let task_queue: String = opts.get("task_queue")?;

        // Collect activity Lua functions → store in registry for cross-thread safety
        let activities_table: Option<Table> = opts.get("activities")?;
        let mut activity_keys: HashMap<String, RegistryKey> = HashMap::new();
        if let Some(tbl) = activities_table {
            for pair in tbl.pairs::<String, Function>() {
                let (name, func) = pair?;
                let key = lua.create_registry_value(func)?;
                activity_keys.insert(name, key);
            }
        }

        // Collect workflow Lua functions
        let workflows_table: Option<Table> = opts.get("workflows")?;
        let mut workflow_keys: HashMap<String, RegistryKey> = HashMap::new();
        if let Some(tbl) = workflows_table {
            for pair in tbl.pairs::<String, Function>() {
                let (name, func) = pair?;
                let key = lua.create_registry_value(func)?;
                workflow_keys.insert(name, key);
            }
        }

        if activity_keys.is_empty() && workflow_keys.is_empty() {
            return Err(mlua::Error::runtime(
                "temporal.worker: at least one activity or workflow must be registered",
            ));
        }

        // Determine task types
        let task_types = match (activity_keys.is_empty(), workflow_keys.is_empty()) {
            (false, false) => WorkerTaskTypes::all(),
            (false, true) => WorkerTaskTypes::activity_only(),
            (true, false) => WorkerTaskTypes::workflow_only(),
            (true, true) => unreachable!(),
        };

        // Connect to Temporal
        let parsed_url = url::Url::parse(&format!("http://{url}"))
            .map_err(|e| mlua::Error::runtime(format!("invalid temporal URL: {e}")))?;
        let connection = Connection::connect(ConnectionOptions::new(parsed_url).build())
            .await
            .map_err(|e| mlua::Error::runtime(format!("temporal worker connect: {e}")))?;

        // Create core runtime and worker
        let telemetry = temporalio_common::telemetry::TelemetryOptions::builder().build();
        let runtime_opts = RuntimeOptions::builder()
            .telemetry_options(telemetry)
            .build()
            .map_err(|e| mlua::Error::runtime(format!("temporal runtime: {e}")))?;
        let runtime = CoreRuntime::new_assume_tokio(runtime_opts)
            .map_err(|e| mlua::Error::runtime(format!("temporal runtime init: {e}")))?;

        let worker_config = WorkerConfig::builder()
            .namespace(namespace.clone())
            .task_queue(task_queue.clone())
            .task_types(task_types)
            .workflow_task_poller_behavior(PollerBehavior::SimpleMaximum(2))
            .activity_task_poller_behavior(PollerBehavior::SimpleMaximum(5))
            .versioning_strategy(WorkerVersioningStrategy::WorkerDeploymentBased(
                WorkerDeploymentOptions::from_build_id("assay-lua-worker".to_owned()),
            ))
            .build()
            .map_err(|e| mlua::Error::runtime(format!("temporal worker config: {e}")))?;

        let core_worker = Arc::new(
            init_worker(&runtime, worker_config, connection)
                .map_err(|e| mlua::Error::runtime(format!("temporal worker init: {e}")))?,
        );

        let shutdown = Arc::new(AtomicBool::new(false));

        // --- Activity polling loop ---
        if !activity_keys.is_empty() {
            let worker = core_worker.clone();
            // Activity names for logging
            let act_names: Vec<String> = activity_keys.keys().cloned().collect();
            tracing::info!("temporal worker: registered activities: {:?}", act_names);

            // Store activity registry keys in a shared structure.
            // Lua functions must be called from the Lua thread, so we use
            // a channel to send tasks back to the main Lua async runtime.
            let (act_tx, mut act_rx) =
                tokio::sync::mpsc::unbounded_channel::<(String, Vec<Payload>, Vec<u8>)>();
            let (result_tx, mut result_rx) =
                tokio::sync::mpsc::unbounded_channel::<(Vec<u8>, ActivityExecutionResult)>();

            // Poller: runs on a tokio task, sends activity tasks to the Lua dispatcher
            let act_tx_clone = act_tx.clone();
            let worker_poll = worker.clone();
            tokio::spawn(async move {
                loop {
                    let task = match worker_poll.poll_activity_task().await {
                        Err(PollError::ShutDown) => break,
                        Err(e) => {
                            tracing::error!("temporal activity poll error: {e}");
                            continue;
                        }
                        Ok(task) => task,
                    };
                    if let Some(activity_task::activity_task::Variant::Start(start)) = task.variant {
                        let _ = act_tx_clone.send((
                            start.activity_type.clone(),
                            start.input,
                            task.task_token,
                        ));
                    }
                }
            });

            // Completer: sends results back to Temporal
            let worker_complete = worker.clone();
            tokio::spawn(async move {
                while let Some((task_token, result)) = result_rx.recv().await {
                    let completion = ActivityTaskCompletion {
                        task_token,
                        result: Some(result),
                    };
                    if let Err(e) = worker_complete.complete_activity_task(completion).await {
                        tracing::error!("temporal activity complete error: {e}");
                    }
                }
            });

            // Lua dispatcher: runs activity Lua functions on the Lua async runtime
            // This must run in the same async context as the Lua VM.
            // Lua is Clone (all clones share the same underlying VM).
            let lua_clone = lua.clone();
            tokio::task::spawn_local(async move {
                while let Some((activity_type, input_payloads, task_token)) = act_rx.recv().await {
                    let lua = &lua_clone;

                    // Look up the Lua function
                    let result = (|| -> Result<Payload, String> {
                        let key = activity_keys
                            .get(&activity_type)
                            .ok_or_else(|| format!("no activity registered: {activity_type}"))?;
                        let func: Function = lua
                            .registry_value(key)
                            .map_err(|e| format!("registry lookup failed: {e}"))?;

                        // Deserialize first input payload as JSON → Lua value
                        let lua_input: Value = if let Some(p) = input_payloads.first() {
                            let json_str =
                                payload_to_json(p).map_err(|e| format!("payload decode: {e}"))?;
                            let json_mod: Table = lua
                                .globals()
                                .get("json")
                                .map_err(|e| format!("json module: {e}"))?;
                            let decode: Function = json_mod
                                .get("decode")
                                .map_err(|e| format!("json.decode: {e}"))?;
                            decode
                                .call(json_str)
                                .map_err(|e| format!("json decode: {e}"))?
                        } else {
                            Value::Nil
                        };

                        // Call the Lua activity function
                        let lua_result: Value = func
                            .call(lua_input)
                            .map_err(|e| format!("activity {activity_type} failed: {e}"))?;

                        // Serialize result to JSON payload
                        let json_mod: Table = lua
                            .globals()
                            .get("json")
                            .map_err(|e| format!("json module: {e}"))?;
                        let encode: Function = json_mod
                            .get("encode")
                            .map_err(|e| format!("json.encode: {e}"))?;
                        let json_str: String = encode
                            .call(lua_result)
                            .map_err(|e| format!("json encode: {e}"))?;

                        Ok(json_payload(&json_str))
                    })();

                    let exec_result = match result {
                        Ok(payload) => ActivityExecutionResult::ok(payload),
                        Err(msg) => {
                            tracing::error!("temporal activity error: {msg}");
                            ActivityExecutionResult::fail(Failure::application_failure(msg, false))
                        }
                    };

                    let _ = result_tx.send((task_token, exec_result));
                }
            });
        }

        // --- Workflow polling loop ---
        // TODO: Implement workflow dispatch with Lua coroutine bridge.
        // For now, workflows are tracked but the actual execution requires
        // the full coroutine-based ctx bridge (next iteration).
        if !workflow_keys.is_empty() {
            let wf_names: Vec<String> = workflow_keys.keys().cloned().collect();
            tracing::info!("temporal worker: registered workflows: {:?}", wf_names);
            tracing::warn!(
                "temporal worker: workflow execution via Lua coroutines is not yet implemented. \
                 Activities are fully functional. Workflow support coming in next release."
            );
        }

        tracing::info!(
            "temporal worker started: task_queue={}, namespace={}",
            task_queue,
            namespace,
        );

        // Return a handle
        let handle = lua.create_table()?;
        let w = core_worker.clone();
        let shutdown_flag = shutdown.clone();
        handle.set(
            "shutdown",
            lua.create_function(move |_, ()| {
                w.initiate_shutdown();
                shutdown_flag.store(true, Ordering::SeqCst);
                Ok(())
            })?,
        )?;
        let shutdown_flag2 = shutdown.clone();
        handle.set(
            "is_running",
            lua.create_function(move |_, ()| {
                Ok(!shutdown_flag2.load(Ordering::SeqCst))
            })?,
        )?;

        Ok(handle)
    })?;
    temporal.set("worker", worker_fn)?;

    Ok(())
}

#[cfg(not(feature = "temporal"))]
pub fn register_temporal_worker(_lua: &mlua::Lua) -> mlua::Result<()> {
    Ok(())
}
