// Regression for issue #40 — older assay versions failed at coroutine
// resume with `error converting Lua nil to function` when a workflow
// handler called `os.date()` or `ctx:register_query()`. The v0.13.0
// rewrite moved workflow execution to pure-Lua `coroutine.create` (in
// `stdlib/engine/workflow/worker.lua`), which inherits globals from the
// parent state. This test pins that contract by exercising the same
// code paths the bug reported, without requiring a running engine.

mod common;

use common::run_lua;

#[tokio::test]
async fn workflow_coroutine_can_call_os_date_and_register_query() {
    let script = r#"
        -- Simulate the ctx the worker hands to a workflow function.
        -- register_query in the real ctx stores into _query_handlers; the
        -- shape mirrors stdlib/engine/workflow/ctx.lua.
        local ctx = {
            _query_handlers = {},
        }
        function ctx:register_query(name, fn)
            self._query_handlers[name] = fn
        end

        -- This is the exact pattern from the issue: a workflow function
        -- that calls register_query and os.date on first run.
        local function workflow(c, input)
            c:register_query("state", function() return { ok = true } end)
            local now = os.date("!%Y-%m-%dT%H:%M:%SZ")
            return { now = now, input = input }
        end

        local co = coroutine.create(function() return workflow(ctx, { hello = true }) end)
        local ok, result = coroutine.resume(co)

        assert.eq(ok, true)
        assert.eq(coroutine.status(co), "dead")
        assert.eq(type(result), "table")
        assert.eq(result.input.hello, true)
        assert.eq(result.now ~= nil and #result.now > 0, true)
        assert.not_nil(ctx._query_handlers.state)

        -- Query handler is callable from outside the coroutine.
        local snapshot = ctx._query_handlers.state()
        assert.eq(snapshot.ok, true)
    "#;
    run_lua(script).await.unwrap();
}

#[tokio::test]
async fn coroutine_can_yield_and_resume_with_args() {
    // Workflow ctx commands work via coroutine.yield -> host resumes with
    // the result. Verify that round-trip doesn't hit the "nil to function"
    // class of errors the issue described.
    let script = r#"
        local function workflow()
            local first = coroutine.yield({ type = "Step1" })
            local second = coroutine.yield({ type = "Step2", got = first })
            return { final = second }
        end

        local co = coroutine.create(workflow)
        local ok, cmd1 = coroutine.resume(co)
        assert.eq(ok, true)
        assert.eq(cmd1.type, "Step1")

        local ok2, cmd2 = coroutine.resume(co, "alpha")
        assert.eq(ok2, true)
        assert.eq(cmd2.type, "Step2")
        assert.eq(cmd2.got, "alpha")

        local ok3, result = coroutine.resume(co, "beta")
        assert.eq(ok3, true)
        assert.eq(result.final, "beta")
        assert.eq(coroutine.status(co), "dead")
    "#;
    run_lua(script).await.unwrap();
}
