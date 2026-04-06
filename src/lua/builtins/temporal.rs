/// Temporal workflow engine — native gRPC bridge for Lua.
///
/// Provides `temporal.connect(opts)` and `temporal.start(opts)`.
///
/// Usage from Lua:
///   local client = temporal.connect({
///     url = "temporal-frontend:7233",
///     namespace = "command-center",
///   })
///   local handle = client:start_workflow({
///     task_queue = "promotions",
///     workflow_type = "promote",
///     workflow_id = "promote-prod-v0.2.0",
///     input = { version = "v0.2.0", target = "prod" },
///   })
#[cfg(feature = "temporal")]
pub fn register_temporal(lua: &mlua::Lua) -> mlua::Result<()> {
    use mlua::{Table, UserData, UserDataMethods, Value};
    use temporalio_client::{
        Client, ClientOptions, Connection, ConnectionOptions, UntypedQuery, UntypedSignal,
        UntypedWorkflow, WorkflowCancelOptions, WorkflowDescribeOptions, WorkflowGetResultOptions,
        WorkflowQueryOptions, WorkflowSignalOptions, WorkflowStartOptions,
        WorkflowTerminateOptions,
    };
    use temporalio_common::{
        data_converters::RawValue,
        protos::temporal::api::common::v1::Payload,
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

    fn workflow_status_str(status: i32) -> &'static str {
        match status {
            1 => "RUNNING",
            2 => "COMPLETED",
            3 => "FAILED",
            4 => "CANCELED",
            5 => "TERMINATED",
            6 => "CONTINUED_AS_NEW",
            7 => "TIMED_OUT",
            _ => "UNKNOWN",
        }
    }

    struct TemporalClient {
        client: Client,
    }

    impl UserData for TemporalClient {
        fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
            // client:start_workflow({ task_queue, workflow_type, workflow_id, input? })
            methods.add_async_method("start_workflow", |lua, this, opts: Table| {
                let client = this.client.clone();
                async move {
                    let task_queue: String = opts.get("task_queue")?;
                    let workflow_type: String = opts.get("workflow_type")?;
                    let workflow_id: String = opts.get("workflow_id")?;
                    let input: Option<Value> = opts.get("input")?;

                    let input_raw = if let Some(val) = input {
                        let json_mod: Table = lua.globals().get("json")?;
                        let encode: mlua::Function = json_mod.get("encode")?;
                        let json_str: String = encode.call_async(val).await?;
                        RawValue::new(vec![json_payload(&json_str)])
                    } else {
                        RawValue::empty()
                    };

                    let handle = client
                        .start_workflow(
                            UntypedWorkflow::new(&workflow_type),
                            input_raw,
                            WorkflowStartOptions::new(&task_queue, &workflow_id).build(),
                        )
                        .await
                        .map_err(|e| mlua::Error::runtime(format!("temporal start_workflow: {e}")))?;

                    let result = lua.create_table()?;
                    result.set("workflow_id", handle.info().workflow_id.clone())?;
                    result.set("run_id", handle.info().run_id.clone().unwrap_or_default())?;
                    Ok(result)
                }
            });

            // client:signal_workflow({ workflow_id, signal_name, input? })
            methods.add_async_method("signal_workflow", |lua, this, opts: Table| {
                let client = this.client.clone();
                async move {
                    let workflow_id: String = opts.get("workflow_id")?;
                    let signal_name: String = opts.get("signal_name")?;
                    let input: Option<Value> = opts.get("input")?;

                    let input_raw = if let Some(val) = input {
                        let json_mod: Table = lua.globals().get("json")?;
                        let encode: mlua::Function = json_mod.get("encode")?;
                        let json_str: String = encode.call_async(val).await?;
                        RawValue::new(vec![json_payload(&json_str)])
                    } else {
                        RawValue::empty()
                    };

                    let handle = client
                        .get_workflow_handle::<UntypedWorkflow>(&workflow_id);
                    handle
                        .signal(
                            UntypedSignal::<UntypedWorkflow>::new(&signal_name),
                            input_raw,
                            WorkflowSignalOptions::default(),
                        )
                        .await
                        .map_err(|e| mlua::Error::runtime(format!("temporal signal: {e}")))?;

                    Ok(Value::Nil)
                }
            });

            // client:query_workflow({ workflow_id, query_type, input? })
            methods.add_async_method("query_workflow", |lua, this, opts: Table| {
                let client = this.client.clone();
                async move {
                    let workflow_id: String = opts.get("workflow_id")?;
                    let query_type: String = opts.get("query_type")?;
                    let input: Option<Value> = opts.get("input")?;

                    let input_raw = if let Some(val) = input {
                        let json_mod: Table = lua.globals().get("json")?;
                        let encode: mlua::Function = json_mod.get("encode")?;
                        let json_str: String = encode.call_async(val).await?;
                        RawValue::new(vec![json_payload(&json_str)])
                    } else {
                        RawValue::empty()
                    };

                    let handle = client
                        .get_workflow_handle::<UntypedWorkflow>(&workflow_id);
                    let raw_result = handle
                        .query(
                            UntypedQuery::<UntypedWorkflow>::new(&query_type),
                            input_raw,
                            WorkflowQueryOptions::default(),
                        )
                        .await
                        .map_err(|e| mlua::Error::runtime(format!("temporal query: {e}")))?;

                    // Decode first payload as JSON
                    if let Some(payload) = raw_result.payloads.first() {
                        let json_str = std::str::from_utf8(&payload.data)
                            .map_err(|e| mlua::Error::runtime(format!("temporal query result: {e}")))?;
                        let json_mod: Table = lua.globals().get("json")?;
                        let decode: mlua::Function = json_mod.get("decode")?;
                        decode.call_async(json_str.to_string()).await
                    } else {
                        Ok(Value::Nil)
                    }
                }
            });

            // client:describe_workflow(workflow_id) or client:describe_workflow({ workflow_id })
            methods.add_async_method("describe_workflow", |lua, this, arg: Value| {
                let client = this.client.clone();
                async move {
                    let workflow_id: String = match arg {
                        Value::String(s) => s.to_str()?.to_string(),
                        Value::Table(t) => t.get("workflow_id")?,
                        _ => return Err(mlua::Error::runtime("expected workflow_id string or table")),
                    };

                    let handle = client
                        .get_workflow_handle::<UntypedWorkflow>(&workflow_id);
                    let desc = handle
                        .describe(WorkflowDescribeOptions::default())
                        .await
                        .map_err(|e| mlua::Error::runtime(format!("temporal describe: {e}")))?;

                    let result = lua.create_table()?;
                    result.set("workflow_id", workflow_id)?;

                    if let Some(info) = &desc.raw_description.workflow_execution_info {
                        result.set("status", workflow_status_str(info.status))?;
                        result.set("history_length", info.history_length)?;
                        if let Some(exec) = &info.execution {
                            result.set("run_id", exec.run_id.clone())?;
                        }
                        if let Some(wf_type) = &info.r#type {
                            result.set("workflow_type", wf_type.name.clone())?;
                        }
                        if let Some(ts) = &info.start_time {
                            result.set("start_time", ts.seconds as f64 + ts.nanos as f64 / 1e9)?;
                        }
                        if let Some(ts) = &info.close_time {
                            result.set("close_time", ts.seconds as f64 + ts.nanos as f64 / 1e9)?;
                        }
                    }

                    Ok(result)
                }
            });

            // client:get_result({ workflow_id, follow_runs? })
            methods.add_async_method("get_result", |lua, this, opts: Table| {
                let client = this.client.clone();
                async move {
                    let workflow_id: String = opts.get("workflow_id")?;
                    let follow_runs: bool = opts.get::<Option<bool>>("follow_runs")?.unwrap_or(true);

                    let handle = client
                        .get_workflow_handle::<UntypedWorkflow>(&workflow_id);
                    let raw_result = handle
                        .get_result(
                            WorkflowGetResultOptions::builder()
                                .follow_runs(follow_runs)
                                .build(),
                        )
                        .await
                        .map_err(|e| mlua::Error::runtime(format!("temporal get_result: {e}")))?;

                    // Decode first payload as JSON
                    if let Some(payload) = raw_result.payloads.first() {
                        let json_str = std::str::from_utf8(&payload.data)
                            .map_err(|e| mlua::Error::runtime(format!("temporal result: {e}")))?;
                        let json_mod: Table = lua.globals().get("json")?;
                        let decode: mlua::Function = json_mod.get("decode")?;
                        decode.call_async(json_str.to_string()).await
                    } else {
                        Ok(Value::Nil)
                    }
                }
            });

            // client:cancel_workflow(workflow_id) or client:cancel_workflow({ workflow_id, reason? })
            methods.add_async_method("cancel_workflow", |_lua, this, arg: Value| {
                let client = this.client.clone();
                async move {
                    let (workflow_id, reason) = match arg {
                        Value::String(s) => (s.to_str()?.to_string(), String::new()),
                        Value::Table(t) => (
                            t.get::<String>("workflow_id")?,
                            t.get::<Option<String>>("reason")?.unwrap_or_default(),
                        ),
                        _ => return Err(mlua::Error::runtime("expected workflow_id string or table")),
                    };

                    let handle = client
                        .get_workflow_handle::<UntypedWorkflow>(&workflow_id);
                    handle
                        .cancel(WorkflowCancelOptions::builder().reason(reason).build())
                        .await
                        .map_err(|e| mlua::Error::runtime(format!("temporal cancel: {e}")))?;

                    Ok(Value::Nil)
                }
            });

            // client:terminate_workflow(workflow_id) or client:terminate_workflow({ workflow_id, reason? })
            methods.add_async_method("terminate_workflow", |_lua, this, arg: Value| {
                let client = this.client.clone();
                async move {
                    let (workflow_id, reason) = match arg {
                        Value::String(s) => (s.to_str()?.to_string(), String::new()),
                        Value::Table(t) => (
                            t.get::<String>("workflow_id")?,
                            t.get::<Option<String>>("reason")?.unwrap_or_default(),
                        ),
                        _ => return Err(mlua::Error::runtime("expected workflow_id string or table")),
                    };

                    let handle = client
                        .get_workflow_handle::<UntypedWorkflow>(&workflow_id);
                    handle
                        .terminate(WorkflowTerminateOptions::builder().reason(reason).build())
                        .await
                        .map_err(|e| mlua::Error::runtime(format!("temporal terminate: {e}")))?;

                    Ok(Value::Nil)
                }
            });
        }
    }

    // Helper: create a gRPC connection and client
    async fn connect_client(url_str: &str, namespace: &str) -> Result<Client, mlua::Error> {
        let parsed_url = url::Url::parse(&format!("http://{url_str}"))
            .map_err(|e| mlua::Error::runtime(format!("invalid temporal URL: {e}")))?;
        let connection = Connection::connect(ConnectionOptions::new(parsed_url).build())
            .await
            .map_err(|e| mlua::Error::runtime(format!("temporal connect: {e}")))?;
        Client::new(connection, ClientOptions::new(namespace).build())
            .map_err(|e| mlua::Error::runtime(format!("temporal client: {e}")))
    }

    let temporal = lua.create_table()?;

    // temporal.connect({ url, namespace? }) -> TemporalClient userdata
    let connect_fn = lua.create_async_function(|lua, opts: Table| async move {
        let url: String = opts.get("url")?;
        let namespace: String = opts
            .get::<Option<String>>("namespace")?
            .unwrap_or_else(|| "default".to_string());
        let client = connect_client(&url, &namespace).await?;
        lua.create_userdata(TemporalClient { client })
    })?;
    temporal.set("connect", connect_fn)?;

    // temporal.start({ url, namespace?, task_queue, workflow_type, workflow_id, input? })
    // One-shot convenience: connects, starts workflow, returns result
    let start_fn = lua.create_async_function(|lua, opts: Table| async move {
        let url: String = opts.get("url")?;
        let namespace: String = opts
            .get::<Option<String>>("namespace")?
            .unwrap_or_else(|| "default".to_string());
        let task_queue: String = opts.get("task_queue")?;
        let workflow_type: String = opts.get("workflow_type")?;
        let workflow_id: String = opts.get("workflow_id")?;
        let input: Option<Value> = opts.get("input")?;

        let input_raw = if let Some(val) = input {
            let json_mod: Table = lua.globals().get("json")?;
            let encode: mlua::Function = json_mod.get("encode")?;
            let json_str: String = encode.call_async(val).await?;
            RawValue::new(vec![json_payload(&json_str)])
        } else {
            RawValue::empty()
        };

        let client = connect_client(&url, &namespace).await?;
        let handle = client
            .start_workflow(
                UntypedWorkflow::new(&workflow_type),
                input_raw,
                WorkflowStartOptions::new(&task_queue, &workflow_id).build(),
            )
            .await
            .map_err(|e| mlua::Error::runtime(format!("temporal start_workflow: {e}")))?;

        let result = lua.create_table()?;
        result.set("workflow_id", handle.info().workflow_id.clone())?;
        result.set("run_id", handle.info().run_id.clone().unwrap_or_default())?;
        Ok(result)
    })?;
    temporal.set("start", start_fn)?;

    lua.globals().set("temporal", temporal)?;
    Ok(())
}

#[cfg(not(feature = "temporal"))]
pub fn register_temporal(_lua: &mlua::Lua) -> mlua::Result<()> {
    Ok(())
}
