/// Temporal worker runtime — bridges Lua activities and workflows to Temporal via CoreWorker.
///
/// Uses the low-level `temporalio-sdk-core` Worker API directly for full control
/// over activity/workflow dispatch to Lua functions.
///
/// ## Architecture
///
/// ```text
/// temporal.worker(opts)
///   │
///   ├── Activity Loop (tokio task)
///   │     poll_activity_task() → channel → spawn_local Lua dispatch → complete_activity_task()
///   │
///   └── Workflow Loop (tokio task + spawn_local)
///         poll_workflow_activation() → channel → Lua coroutine dispatch
///           → ctx methods yield/resume commands → complete_workflow_activation()
/// ```
///
/// ## Workflow Coroutine Model
///
/// Each workflow execution gets a Lua coroutine. The `ctx` object provides deterministic
/// primitives (`execute_activity`, `wait_signal`, `sleep`, `side_effect`, `workflow_info`).
/// On replay, ctx methods return cached results from the activation's resolved events
/// without yielding, so the coroutine replays to the correct point instantly.
#[cfg(feature = "temporal")]
pub fn register_temporal_worker(lua: &mlua::Lua) -> mlua::Result<()> {
    use mlua::{Function, MultiValue, RegistryKey, Table, Thread, Value};
    use std::collections::{HashMap, HashSet, VecDeque};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use temporalio_client::{Connection, ConnectionOptions};
    use temporalio_common::{
        protos::{
            coresdk::{
                ActivityTaskCompletion,
                activity_result::ActivityExecutionResult,
                activity_task,
                workflow_activation::{self, workflow_activation_job},
                workflow_commands::{self, workflow_command},
                workflow_completion::{self, WorkflowActivationCompletion},
            },
            temporal::api::{
                common::v1::Payload,
                failure::v1::Failure,
            },
        },
        worker::WorkerTaskTypes,
    };
    use temporalio_sdk_core::{
        CoreRuntime, PollError, PollerBehavior, RuntimeOptions, WorkerConfig,
        WorkerVersioningStrategy, init_worker,
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

    /// Lua source for the workflow context factory.
    /// Creates a ctx table with deterministic methods that yield commands to the Rust dispatcher.
    /// On replay, methods consume pre-populated `_resolved` and `_signals` tables instead of
    /// yielding, so the coroutine fast-forwards through already-completed work.
    ///
    /// Sequence numbers (`_next_seq`) are always incremented, even when consuming from replay
    /// buffers, to ensure deterministic command-to-result matching across replays.
    const CTX_LUA: &str = r#"
local function create_workflow_ctx(resolved, signals, info)
    local ctx = {
        _next_seq = 1,
        _resolved = resolved or {},
        _signals = signals or {},
        _info = info or {},
    }

    function ctx:execute_activity(name, input, opts)
        local seq = self._next_seq
        self._next_seq = seq + 1
        local resolved = self._resolved[seq]
        if resolved ~= nil then
            self._resolved[seq] = nil
            if type(resolved) == "table" and resolved._activity_error then
                error("activity '" .. name .. "' failed: " .. resolved._activity_error, 2)
            end
            return resolved
        end
        local result = coroutine.yield({
            type = "schedule_activity",
            seq = seq,
            name = name,
            input = input,
            opts = opts or {},
        })
        if type(result) == "table" and result._activity_error then
            error("activity '" .. name .. "' failed: " .. result._activity_error, 2)
        end
        return result
    end

    function ctx:wait_signal(name, opts)
        local seq = self._next_seq
        self._next_seq = seq + 1
        local buf = self._signals[name]
        if buf and #buf > 0 then
            return table.remove(buf, 1)
        end
        if self._resolved[seq] then
            self._resolved[seq] = nil
            return nil
        end
        local timeout = opts and opts.timeout
        return coroutine.yield({
            type = "wait_signal",
            seq = seq,
            name = name,
            timeout = timeout,
        })
    end

    function ctx:sleep(seconds)
        local seq = self._next_seq
        self._next_seq = seq + 1
        if self._resolved[seq] then
            self._resolved[seq] = nil
            return
        end
        coroutine.yield({ type = "sleep", seq = seq, duration = seconds })
    end

    function ctx:side_effect(fn)
        local seq = self._next_seq
        self._next_seq = seq + 1
        local resolved = self._resolved[seq]
        if resolved ~= nil then
            self._resolved[seq] = nil
            return resolved
        end
        return fn()
    end

    function ctx:workflow_info()
        return self._info
    end

    return ctx
end
return create_workflow_ctx
"#;

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

        // =================================================================
        // Activity polling loop
        // =================================================================
        if !activity_keys.is_empty() {
            let worker = core_worker.clone();
            let act_names: Vec<String> = activity_keys.keys().cloned().collect();
            tracing::info!("temporal worker: registered activities: {:?}", act_names);

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
                    if let Some(activity_task::activity_task::Variant::Start(start)) =
                        task.variant
                    {
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
            let lua_clone = lua.clone();
            tokio::task::spawn_local(async move {
                while let Some((activity_type, input_payloads, task_token)) =
                    act_rx.recv().await
                {
                    let lua = &lua_clone;

                    let result = (|| -> Result<Payload, String> {
                        let key = activity_keys
                            .get(&activity_type)
                            .ok_or_else(|| format!("no activity registered: {activity_type}"))?;
                        let func: Function = lua
                            .registry_value(key)
                            .map_err(|e| format!("registry lookup failed: {e}"))?;

                        // Deserialize first input payload as JSON → Lua value
                        let lua_input: Value = if let Some(p) = input_payloads.first() {
                            let json_str = payload_to_json(p)
                                .map_err(|e| format!("payload decode: {e}"))?;
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
                            ActivityExecutionResult::fail(Failure::application_failure(
                                msg, false,
                            ))
                        }
                    };

                    let _ = result_tx.send((task_token, exec_result));
                }
            });
        }

        // =================================================================
        // Workflow polling loop
        // =================================================================
        if !workflow_keys.is_empty() {
            let worker = core_worker.clone();
            let wf_names: Vec<String> = workflow_keys.keys().cloned().collect();
            tracing::info!("temporal worker: registered workflows: {:?}", wf_names);

            let (wf_tx, mut wf_rx) = tokio::sync::mpsc::unbounded_channel::<
                workflow_activation::WorkflowActivation,
            >();
            let (wf_result_tx, mut wf_result_rx) =
                tokio::sync::mpsc::unbounded_channel::<WorkflowActivationCompletion>();

            // Poller: polls workflow activations from Temporal server
            let worker_poll = worker.clone();
            tokio::spawn(async move {
                loop {
                    match worker_poll.poll_workflow_activation().await {
                        Err(PollError::ShutDown) => break,
                        Err(e) => {
                            tracing::error!("temporal workflow poll error: {e}");
                            continue;
                        }
                        Ok(activation) => {
                            if wf_tx.send(activation).is_err() {
                                break;
                            }
                        }
                    }
                }
            });

            // Completer: sends workflow completions back to Temporal
            let worker_complete = worker.clone();
            tokio::spawn(async move {
                while let Some(completion) = wf_result_rx.recv().await {
                    if let Err(e) = worker_complete
                        .complete_workflow_activation(completion)
                        .await
                    {
                        tracing::error!("temporal workflow complete error: {e}");
                    }
                }
            });

            // Lua workflow dispatcher: processes activations via coroutines
            let lua_clone = lua.clone();
            let wf_task_queue = task_queue.clone();
            let wf_namespace = namespace.clone();
            tokio::task::spawn_local(async move {
                let lua = &lua_clone;

                // Load the ctx factory function once
                let ctx_factory: Function = match lua.load(CTX_LUA).eval() {
                    Ok(f) => f,
                    Err(e) => {
                        tracing::error!("failed to load workflow ctx factory: {e}");
                        return;
                    }
                };
                let ctx_factory_key = match lua.create_registry_value(ctx_factory) {
                    Ok(k) => k,
                    Err(e) => {
                        tracing::error!("failed to store ctx factory: {e}");
                        return;
                    }
                };

                /// Per-workflow instance state, keyed by run_id.
                struct WfInstance {
                    thread_key: RegistryKey,
                    ctx_key: RegistryKey,
                    pending: Option<PendingWait>,
                }

                let mut instances: HashMap<String, WfInstance> = HashMap::new();

                while let Some(activation) = wf_rx.recv().await {
                    let run_id = activation.run_id.clone();
                    let run_id_err = run_id.clone(); // for error logging outside closure

                    // Process the activation — errors are caught and sent as
                    // workflow task failures so the worker stays alive.
                    let completion = (|| -> mlua::Result<WorkflowActivationCompletion> {
                        // --- Eviction: clean up and acknowledge ---
                        if activation.is_only_eviction() {
                            instances.remove(&run_id);
                            tracing::debug!("temporal workflow evicted: {run_id}");
                            return Ok(WorkflowActivationCompletion::empty(run_id));
                        }

                        // --- Decode all activation jobs into typed buffers ---
                        let mut init_job: Option<workflow_activation::InitializeWorkflow> = None;
                        let mut resolved_activities: HashMap<u32, Value> = HashMap::new();
                        let mut signal_buffer: HashMap<String, VecDeque<Value>> = HashMap::new();
                        let mut fired_timers: HashSet<u32> = HashSet::new();
                        let mut cancelled = false;

                        for job in &activation.jobs {
                            match &job.variant {
                                Some(workflow_activation_job::Variant::InitializeWorkflow(
                                    init,
                                )) => {
                                    init_job = Some(init.clone());
                                }
                                Some(workflow_activation_job::Variant::ResolveActivity(
                                    resolve,
                                )) => {
                                    // Decode activity result → Lua value (or error table)
                                    let lua_val =
                                        decode_activity_result(lua, resolve)?;
                                    resolved_activities.insert(resolve.seq, lua_val);
                                }
                                Some(workflow_activation_job::Variant::SignalWorkflow(
                                    signal,
                                )) => {
                                    let payload = if let Some(p) = signal.input.first() {
                                        let json_str = payload_to_json(p)
                                            .map_err(mlua::Error::runtime)?;
                                        let json_mod: Table = lua.globals().get("json")?;
                                        let decode: Function = json_mod.get("decode")?;
                                        decode.call::<Value>(json_str)?
                                    } else {
                                        Value::Nil
                                    };
                                    signal_buffer
                                        .entry(signal.signal_name.clone())
                                        .or_default()
                                        .push_back(payload);
                                }
                                Some(workflow_activation_job::Variant::FireTimer(timer)) => {
                                    fired_timers.insert(timer.seq);
                                }
                                Some(workflow_activation_job::Variant::CancelWorkflow(_)) => {
                                    cancelled = true;
                                }
                                Some(workflow_activation_job::Variant::RemoveFromCache(_)) => {
                                    // Already handled by is_only_eviction() above;
                                    // if combined with other jobs, ignore here.
                                }
                                _ => {
                                    tracing::debug!(
                                        "temporal workflow {run_id}: unhandled activation job"
                                    );
                                }
                            }
                        }

                        // =================================================
                        // InitializeWorkflow — create a new coroutine
                        // =================================================
                        if let Some(init) = init_job {
                            let workflow_type = &init.workflow_type;
                            let wf_key =
                                workflow_keys.get(workflow_type).ok_or_else(|| {
                                    mlua::Error::runtime(format!(
                                        "no workflow registered: {workflow_type}"
                                    ))
                                })?;
                            let wf_func: Function = lua.registry_value(wf_key)?;

                            // Build workflow_info table
                            let info = lua.create_table()?;
                            info.set("workflow_id", init.workflow_id.as_str())?;
                            info.set("workflow_type", workflow_type.as_str())?;
                            info.set("attempt", init.attempt)?;
                            info.set("task_queue", wf_task_queue.as_str())?;
                            info.set("namespace", wf_namespace.as_str())?;
                            if let Some(ts) = &init.start_time {
                                info.set(
                                    "start_time",
                                    ts.seconds as f64 + ts.nanos as f64 / 1e9,
                                )?;
                            }

                            // Pre-populate replay buffers
                            let resolved_table = lua.create_table()?;
                            let signals_table = lua.create_table()?;

                            for (seq, val) in &resolved_activities {
                                resolved_table.set(*seq, val.clone())?;
                            }
                            for seq in &fired_timers {
                                resolved_table.set(*seq, true)?;
                            }
                            for (name, payloads) in &signal_buffer {
                                let buf = lua.create_table()?;
                                for (i, p) in payloads.iter().enumerate() {
                                    buf.set(i + 1, p.clone())?;
                                }
                                signals_table.set(name.as_str(), buf)?;
                            }

                            // Create ctx via the Lua factory
                            let ctx_factory: Function =
                                lua.registry_value(&ctx_factory_key)?;
                            let ctx: Table =
                                ctx_factory.call((resolved_table, signals_table, info))?;

                            // Decode workflow input
                            let input = if let Some(p) = init.arguments.first() {
                                let json_str =
                                    payload_to_json(p).map_err(mlua::Error::runtime)?;
                                let json_mod: Table = lua.globals().get("json")?;
                                let decode: Function = json_mod.get("decode")?;
                                decode.call::<Value>(json_str)?
                            } else {
                                Value::Nil
                            };

                            // Create coroutine from workflow function
                            let thread = lua.create_thread(wf_func)?;

                            // Resume with (ctx, input) — the workflow function signature
                            let result =
                                thread.resume::<MultiValue>((ctx.clone(), input));
                            let (commands, pending) = process_coroutine_result(
                                lua,
                                &thread,
                                result,
                                &wf_task_queue,
                            )?;

                            // Store the instance for future activations
                            let ctx_key = lua.create_registry_value(ctx)?;
                            let thread_key = lua.create_registry_value(thread)?;
                            instances.insert(
                                run_id.clone(),
                                WfInstance {
                                    thread_key,
                                    ctx_key,
                                    pending,
                                },
                            );

                            return Ok(WorkflowActivationCompletion::from_cmds(
                                run_id, commands,
                            ));
                        }

                        // =================================================
                        // CancelWorkflow — cancel and clean up
                        // =================================================
                        if cancelled {
                            instances.remove(&run_id);
                            return Ok(WorkflowActivationCompletion::from_cmd(
                                run_id,
                                workflow_command::Variant::CancelWorkflowExecution(
                                    workflow_commands::CancelWorkflowExecution {},
                                ),
                            ));
                        }

                        // =================================================
                        // Resume existing workflow coroutine
                        // =================================================
                        let instance =
                            instances.get_mut(&run_id).ok_or_else(|| {
                                mlua::Error::runtime(format!(
                                    "no workflow instance for run_id: {run_id}"
                                ))
                            })?;

                        let ctx: Table = lua.registry_value(&instance.ctx_key)?;
                        let thread: Thread = lua.registry_value(&instance.thread_key)?;

                        // Determine resume value from the pending wait, consuming
                        // the matching item from the buffers.
                        let mut extra_commands: Vec<workflow_command::Variant> = vec![];
                        let resume_val: Value = match &instance.pending {
                            Some(PendingWait::Activity { seq }) => {
                                resolved_activities.remove(seq).unwrap_or(Value::Nil)
                            }
                            Some(PendingWait::Signal { name, timer_seq }) => {
                                if let Some(buf) = signal_buffer.get_mut(name) {
                                    if let Some(payload) = buf.pop_front() {
                                        // Signal arrived — cancel the timeout timer
                                        if let Some(ts) = timer_seq {
                                            extra_commands.push(
                                                workflow_command::Variant::CancelTimer(
                                                    workflow_commands::CancelTimer {
                                                        seq: *ts,
                                                    },
                                                ),
                                            );
                                            fired_timers.remove(ts);
                                        }
                                        payload
                                    } else if timer_seq
                                        .map(|s| fired_timers.remove(&s))
                                        .unwrap_or(false)
                                    {
                                        Value::Nil // timeout expired
                                    } else {
                                        // Neither signal nor timer — nothing to resume
                                        return Ok(WorkflowActivationCompletion::empty(
                                            run_id,
                                        ));
                                    }
                                } else if timer_seq
                                    .map(|s| fired_timers.remove(&s))
                                    .unwrap_or(false)
                                {
                                    Value::Nil // timeout expired
                                } else {
                                    return Ok(WorkflowActivationCompletion::empty(run_id));
                                }
                            }
                            Some(PendingWait::Timer { seq }) => {
                                if fired_timers.remove(seq) {
                                    Value::Nil
                                } else {
                                    return Ok(WorkflowActivationCompletion::empty(run_id));
                                }
                            }
                            None => Value::Nil,
                        };

                        // Populate ctx replay tables with REMAINING buffered items
                        // (the consumed item was removed above)
                        let resolved_tbl: Table = ctx.get("_resolved")?;
                        let signals_tbl: Table = ctx.get("_signals")?;

                        for (seq, val) in resolved_activities {
                            resolved_tbl.set(seq, val)?;
                        }
                        for seq in fired_timers {
                            resolved_tbl.set(seq, true)?;
                        }
                        for (name, payloads) in signal_buffer {
                            if payloads.is_empty() {
                                continue;
                            }
                            let buf: Table =
                                match signals_tbl.get::<Option<Table>>(name.as_str())? {
                                    Some(existing) => existing,
                                    None => {
                                        let t = lua.create_table()?;
                                        signals_tbl.set(name.as_str(), t.clone())?;
                                        t
                                    }
                                };
                            for p in payloads {
                                let len = buf.raw_len();
                                buf.set(len + 1, p)?;
                            }
                        }

                        // Resume the coroutine
                        let result = thread.resume::<MultiValue>(resume_val);
                        let (mut commands, pending) =
                            process_coroutine_result(lua, &thread, result, &wf_task_queue)?;
                        instance.pending = pending;

                        // Prepend extra commands (e.g. CancelTimer for signal+timeout)
                        if !extra_commands.is_empty() {
                            extra_commands.extend(commands);
                            commands = extra_commands;
                        }

                        Ok(WorkflowActivationCompletion::from_cmds(run_id, commands))
                    })();

                    match completion {
                        Ok(comp) => {
                            let _ = wf_result_tx.send(comp);
                        }
                        Err(e) => {
                            tracing::error!(
                                "temporal workflow dispatch error for {run_id_err}: {e}"
                            );
                            // Report as a workflow task failure so the server retries
                            let comp = WorkflowActivationCompletion {
                                run_id: run_id_err.clone(),
                                status: Some(
                                    workflow_completion::workflow_activation_completion::Status::Failed(
                                        workflow_completion::Failure {
                                            failure: Some(Failure::application_failure(
                                                e.to_string(),
                                                false,
                                            )),
                                            force_cause: 0,
                                        },
                                    ),
                                ),
                            };
                            let _ = wf_result_tx.send(comp);
                        }
                    }
                }
            });
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
            lua.create_function(move |_, ()| Ok(!shutdown_flag2.load(Ordering::SeqCst)))?,
        )?;

        Ok(handle)
    })?;
    temporal.set("worker", worker_fn)?;

    Ok(())
}

/// Decode an activity resolution from a workflow activation into a Lua value.
/// Returns the decoded result on success, or a table with `_activity_error` on failure.
#[cfg(feature = "temporal")]
fn decode_activity_result(
    lua: &mlua::Lua,
    resolve: &temporalio_common::protos::coresdk::workflow_activation::ResolveActivity,
) -> mlua::Result<mlua::Value> {
    use temporalio_common::protos::coresdk::activity_result;
    use temporalio_common::protos::temporal::api::common::v1::Payload;

    fn payload_to_json(payload: &Payload) -> Result<String, String> {
        std::str::from_utf8(&payload.data)
            .map(|s| s.to_string())
            .map_err(|e| format!("invalid UTF-8 in payload: {e}"))
    }

    let resolution = resolve
        .result
        .as_ref()
        .ok_or_else(|| mlua::Error::runtime("missing activity resolution"))?;

    match &resolution.status {
        Some(activity_result::activity_resolution::Status::Completed(success)) => {
            if let Some(payload) = success.result.as_ref() {
                let json_str = payload_to_json(payload).map_err(mlua::Error::runtime)?;
                let json_mod: mlua::Table = lua.globals().get("json")?;
                let decode: mlua::Function = json_mod.get("decode")?;
                decode.call::<mlua::Value>(json_str)
            } else {
                Ok(mlua::Value::Nil)
            }
        }
        Some(activity_result::activity_resolution::Status::Failed(failure)) => {
            let msg = failure
                .failure
                .as_ref()
                .map(|f| f.message.clone())
                .unwrap_or_else(|| "unknown activity failure".to_string());
            let err_table = lua.create_table()?;
            err_table.set("_activity_error", msg)?;
            Ok(mlua::Value::Table(err_table))
        }
        Some(activity_result::activity_resolution::Status::Cancelled(cancel)) => {
            let msg = cancel
                .failure
                .as_ref()
                .map(|f| f.message.clone())
                .unwrap_or_else(|| "activity cancelled".to_string());
            let err_table = lua.create_table()?;
            err_table.set("_activity_error", msg)?;
            Ok(mlua::Value::Table(err_table))
        }
        _ => {
            let err_table = lua.create_table()?;
            err_table.set("_activity_error", "unexpected activity resolution status")?;
            Ok(mlua::Value::Table(err_table))
        }
    }
}

/// Process the result of resuming a workflow coroutine.
/// Parses yielded command tables into Temporal workflow commands, or creates
/// completion/failure commands when the coroutine finishes or errors.
#[cfg(feature = "temporal")]
fn process_coroutine_result(
    lua: &mlua::Lua,
    thread: &mlua::Thread,
    resume_result: mlua::Result<mlua::MultiValue>,
    task_queue: &str,
) -> mlua::Result<(
    Vec<temporalio_common::protos::coresdk::workflow_commands::workflow_command::Variant>,
    Option<PendingWait>,
)> {
    use temporalio_common::protos::{
        coresdk::workflow_commands::{self, workflow_command},
        temporal::api::{
            common::v1::{Payload, RetryPolicy},
            failure::v1::Failure,
        },
    };

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

    fn duration_secs(seconds: f64) -> prost_wkt_types::Duration {
        prost_wkt_types::Duration {
            seconds: seconds as i64,
            nanos: ((seconds.fract()) * 1_000_000_000.0) as i32,
        }
    }

    match resume_result {
        Err(e) => {
            // Coroutine errored — the workflow function threw
            Ok((
                vec![workflow_command::Variant::FailWorkflowExecution(
                    workflow_commands::FailWorkflowExecution {
                        failure: Some(Failure::application_failure(e.to_string(), false)),
                    },
                )],
                None,
            ))
        }
        Ok(values) => {
            match thread.status() {
                mlua::ThreadStatus::Resumable => {
                    // Coroutine yielded a command table
                    let cmd_table: mlua::Table = values
                        .into_iter()
                        .next()
                        .and_then(|v| match v {
                            mlua::Value::Table(t) => Some(t),
                            _ => None,
                        })
                        .ok_or_else(|| {
                            mlua::Error::runtime("workflow yielded non-table value")
                        })?;

                    let cmd_type: String = cmd_table.get("type")?;
                    let seq: u32 = cmd_table.get("seq")?;

                    match cmd_type.as_str() {
                        // -------------------------------------------------
                        // ctx:execute_activity → ScheduleActivity command
                        // -------------------------------------------------
                        "schedule_activity" => {
                            let name: String = cmd_table.get("name")?;
                            let input: mlua::Value = cmd_table.get("input")?;
                            let opts: Option<mlua::Table> = cmd_table.get("opts")?;

                            // Encode input as JSON payload
                            let mut arguments = vec![];
                            if !input.is_nil() {
                                let json_mod: mlua::Table = lua.globals().get("json")?;
                                let encode: mlua::Function = json_mod.get("encode")?;
                                let json_str: String = encode.call(input)?;
                                arguments.push(json_payload(&json_str));
                            }

                            // Default timeout: 5 minutes
                            let mut start_to_close = duration_secs(300.0);
                            let mut schedule_to_close = None;
                            let mut heartbeat = None;
                            let mut retry = None;

                            if let Some(ref opts) = opts {
                                if let Ok(t) = opts.get::<f64>("start_to_close_timeout") {
                                    start_to_close = duration_secs(t);
                                }
                                if let Ok(t) = opts.get::<f64>("schedule_to_close_timeout") {
                                    schedule_to_close = Some(duration_secs(t));
                                }
                                if let Ok(t) = opts.get::<f64>("heartbeat_timeout") {
                                    heartbeat = Some(duration_secs(t));
                                }
                                if let Ok(rp) = opts.get::<mlua::Table>("retry_policy") {
                                    let mut policy = RetryPolicy::default();
                                    if let Ok(v) = rp.get::<f64>("initial_interval") {
                                        policy.initial_interval = Some(duration_secs(v));
                                    }
                                    if let Ok(v) = rp.get::<f64>("backoff_coefficient") {
                                        policy.backoff_coefficient = v;
                                    }
                                    if let Ok(v) = rp.get::<f64>("maximum_interval") {
                                        policy.maximum_interval = Some(duration_secs(v));
                                    }
                                    if let Ok(v) = rp.get::<i32>("maximum_attempts") {
                                        policy.maximum_attempts = v;
                                    }
                                    if let Ok(errors) =
                                        rp.get::<mlua::Table>("non_retryable_errors")
                                    {
                                        for s in errors.sequence_values::<String>().flatten() {
                                            policy.non_retryable_error_types.push(s);
                                        }
                                    }
                                    retry = Some(policy);
                                }
                            }

                            let schedule = workflow_commands::ScheduleActivity {
                                seq,
                                activity_id: seq.to_string(),
                                activity_type: name,
                                task_queue: task_queue.to_string(),
                                arguments,
                                start_to_close_timeout: Some(start_to_close),
                                schedule_to_close_timeout: schedule_to_close,
                                heartbeat_timeout: heartbeat,
                                retry_policy: retry,
                                ..Default::default()
                            };

                            Ok((
                                vec![workflow_command::Variant::ScheduleActivity(schedule)],
                                Some(PendingWait::Activity { seq }),
                            ))
                        }

                        // -------------------------------------------------
                        // ctx:wait_signal → optional StartTimer command
                        // -------------------------------------------------
                        "wait_signal" => {
                            let name: String = cmd_table.get("name")?;
                            let timeout: Option<f64> = cmd_table.get("timeout")?;

                            let mut commands = vec![];
                            let timer_seq = if let Some(t) = timeout {
                                commands.push(workflow_command::Variant::StartTimer(
                                    workflow_commands::StartTimer {
                                        seq,
                                        start_to_fire_timeout: Some(duration_secs(t)),
                                    },
                                ));
                                Some(seq)
                            } else {
                                None
                            };

                            Ok((commands, Some(PendingWait::Signal { name, timer_seq })))
                        }

                        // -------------------------------------------------
                        // ctx:sleep → StartTimer command
                        // -------------------------------------------------
                        "sleep" => {
                            let duration: f64 = cmd_table.get("duration")?;
                            Ok((
                                vec![workflow_command::Variant::StartTimer(
                                    workflow_commands::StartTimer {
                                        seq,
                                        start_to_fire_timeout: Some(duration_secs(duration)),
                                    },
                                )],
                                Some(PendingWait::Timer { seq }),
                            ))
                        }

                        other => Err(mlua::Error::runtime(format!(
                            "unknown workflow command type: {other}"
                        ))),
                    }
                }

                // Coroutine finished — workflow complete
                _ => {
                    let return_val =
                        values.into_iter().next().unwrap_or(mlua::Value::Nil);
                    let result_payload = if return_val.is_nil() {
                        None
                    } else {
                        let json_mod: mlua::Table = lua.globals().get("json")?;
                        let encode: mlua::Function = json_mod.get("encode")?;
                        let json_str: String = encode.call(return_val)?;
                        Some(json_payload(&json_str))
                    };

                    Ok((
                        vec![workflow_command::Variant::CompleteWorkflowExecution(
                            workflow_commands::CompleteWorkflowExecution {
                                result: result_payload,
                            },
                        )],
                        None,
                    ))
                }
            }
        }
    }
}

/// What the workflow coroutine is currently waiting for.
/// Used to match incoming activation jobs to the correct resume value.
#[cfg(feature = "temporal")]
enum PendingWait {
    Activity { seq: u32 },
    Signal { name: String, timer_seq: Option<u32> },
    Timer { seq: u32 },
}

#[cfg(not(feature = "temporal"))]
pub fn register_temporal_worker(_lua: &mlua::Lua) -> mlua::Result<()> {
    Ok(())
}
