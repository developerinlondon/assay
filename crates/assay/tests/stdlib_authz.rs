mod common;

use common::run_lua;

const VOCABULARY: &str = r#"
local authz = require("assay.authz")
local eng = authz.engine({
  condition_keys = { ["app:Region"] = { type = "string" } },
  scope_kinds = { "root", "space" },
  default_scope_chain = { { kind = "root", id = "*" } },
  actions = {
    { action = "docs.read" },
    { action = "docs.write", derives_from = "docs.read" },
  },
  grants = {
    {
      subject = { kind = "user", id = "alice" },
      scope = { kind = "root", id = "*" },
      statements = {
        { effect = "allow", actions = { "docs.read" }, resources = { "doc:*" } },
      },
    },
    {
      subject = { kind = "user", id = "alice" },
      scope = { kind = "root", id = "*" },
      statements = {
        { effect = "deny", actions = { "docs.read" }, resources = { "doc:secret" } },
      },
    },
    {
      subject = { kind = "user", id = "bounded" },
      scope = { kind = "root", id = "*" },
      statements = {
        { effect = "allow", actions = { "docs.read" }, resources = { "doc:*" } },
      },
      bounds = {
        { operator = "StringEquals", key = "app:Region", value = "eu-west" },
      },
    },
  },
})
local alice = { { kind = "user", id = "alice" } }
local bounded = { { kind = "user", id = "bounded" } }
"#;

async fn run(body: &str) {
    run_lua(&format!("{VOCABULARY}\n{body}"))
        .await
        .unwrap_or_else(|error| panic!("lua failed: {error}"));
}

#[tokio::test]
async fn an_allow_statement_allows() {
    run(r#"
        local d = eng:check(alice, "docs.read", "doc:42")
        assert.eq(d.allowed, true)
        assert.eq(d.decision, "allow")
        assert.eq(d.reason, "allowed")
    "#)
    .await;
}

#[tokio::test]
async fn a_deny_beats_an_allow_from_another_grant() {
    run(r#"
        local d = eng:check(alice, "docs.read", "doc:secret")
        assert.eq(d.allowed, false)
        assert.eq(d.reason, "explicit_deny")
    "#)
    .await;
}

#[tokio::test]
async fn nothing_granted_denies() {
    run(r#"
        local d = eng:check({ { kind = "user", id = "nobody" } }, "docs.read", "doc:42")
        assert.eq(d.allowed, false)
        assert.eq(d.reason, "no_matching_grant")
    "#)
    .await;
}

#[tokio::test]
async fn a_grant_bound_confines_the_allow_it_carries() {
    run(r#"
        local inside = eng:check(bounded, "docs.read", "doc:42", {
          context = { ["app:Region"] = "eu-west" },
        })
        assert.eq(inside.allowed, true)

        local outside = eng:check(bounded, "docs.read", "doc:42", {
          context = { ["app:Region"] = "us-east" },
        })
        assert.eq(outside.allowed, false)

        local unpopulated = eng:check(bounded, "docs.read", "doc:42")
        assert.eq(unpopulated.allowed, false)
    "#)
    .await;
}

#[tokio::test]
async fn a_statement_naming_a_base_action_covers_what_derives_from_it() {
    run(r#"
        local derived = eng:check(alice, "docs.write", "doc:42")
        assert.eq(derived.allowed, true)
    "#)
    .await;
}

#[tokio::test]
async fn a_malformed_scope_chain_denies_outright() {
    run(r#"
        local d = eng:check(alice, "docs.read", "doc:42", {
          scope_chain = { { kind = "root", id = "*" }, { kind = "galaxy", id = "x" } },
        })
        assert.eq(d.allowed, false)
        assert.eq(d.reason, "undeclared_scope_kind")
    "#)
    .await;
}

#[tokio::test]
async fn validate_refuses_a_condition_the_engine_could_never_evaluate() {
    run(r#"
        local ok = eng:validate({
          { effect = "allow", actions = { "docs.read" }, resources = { "doc:*" },
            conditions = { { operator = "StringEquals", key = "app:Region", value = "eu-west" } } },
        })
        assert.not_nil(ok)

        local bad, err = eng:validate({
          { effect = "allow", actions = { "docs.read" }, resources = { "doc:*" },
            conditions = { { operator = "Bogus", key = "app:Region", value = "eu-west" } } },
        })
        assert.eq(bad, nil)
        assert.contains(err, "unknown operator")

        local unknown_action, action_err = eng:validate({
          { effect = "allow", actions = { "docs.destroy" }, resources = { "doc:*" } },
        })
        assert.eq(unknown_action, nil)
        assert.contains(action_err, "unknown or wildcard action")
    "#)
    .await;
}

#[tokio::test]
async fn describe_serves_the_declared_vocabulary_as_data() {
    run(r#"
        local d = eng:describe()
        assert.eq(d.version, 1)
        assert.eq(#d.actions, 2)
        assert.eq(d.actions[1].action, "docs.read")
        assert.eq(d.actionClosures["docs.read"][1], "docs.write")
        assert.eq(d.conditionKeys["request:Time"].builtIn, true)
        assert.eq(d.conditionKeys["app:Region"].type, "string")
        assert.eq(#d.scopeKinds, 2)
    "#)
    .await;
}

#[tokio::test]
async fn grants_for_lists_what_applies_over_a_chain() {
    run(r#"
        local grants = eng:grants_for(alice)
        assert.eq(#grants, 2)
        local none = eng:grants_for({ { kind = "user", id = "nobody" } })
        assert.eq(#none, 0)
    "#)
    .await;
}

const CONTEXT_SHAPES: &str = r#"
local authz = require("assay.authz")
local eng = authz.engine({
  condition_keys = {
    ["app:Roles"] = { type = "string" },
    ["app:Flag"] = { type = "string" },
    ["app:Str"] = { type = "string" },
  },
  scope_kinds = { "root" },
  default_scope_chain = { { kind = "root", id = "*" } },
  grants = {
    {
      subject = { kind = "user", id = "alice" },
      scope = { kind = "root", id = "*" },
      statements = {
        { effect = "allow", actions = { "docs.read" }, resources = { "*" } },
        { effect = "deny", actions = { "docs.read" }, resources = { "*" },
          conditions = { { operator = "StringNotIn", key = "app:Roles", values = { "admin" } } } },
      },
    },
  },
})
local gated = authz.engine({
  condition_keys = {
    ["app:Flag"] = { type = "string" },
    ["app:Str"] = { type = "string" },
  },
  scope_kinds = { "root" },
  default_scope_chain = { { kind = "root", id = "*" } },
  grants = {
    {
      subject = { kind = "user", id = "alice" },
      scope = { kind = "root", id = "*" },
      statements = {
        { effect = "allow", actions = { "docs.read" }, resources = { "*" },
          conditions = { { operator = "StringLike", key = "app:Flag", value = "tr*" } } },
      },
    },
  },
})
local alice = { { kind = "user", id = "alice" } }
"#;

async fn run_shapes(body: &str) {
    run_lua(&format!("{CONTEXT_SHAPES}\n{body}"))
        .await
        .unwrap_or_else(|error| panic!("lua failed: {error}"));
}

#[tokio::test]
async fn an_empty_context_table_decides_instead_of_aborting_the_check() {
    run_shapes(
        r#"
        local held_none = eng:check(alice, "docs.read", "doc:1", {
          context = { ["app:Roles"] = {} },
        })
        assert.eq(held_none.allowed, false)
        assert.eq(held_none.reason, "explicit_deny")

        local held_admin = eng:check(alice, "docs.read", "doc:1", {
          context = { ["app:Roles"] = { "admin" } },
        })
        assert.eq(held_admin.allowed, true)
    "#,
    )
    .await;
}

#[tokio::test]
async fn a_sparse_context_table_decides_instead_of_aborting_the_check() {
    run_shapes(
        r#"
        local sparse = eng:check(alice, "docs.read", "doc:1", {
          context = { ["app:Roles"] = { "admin", nil, "sre" } },
        })
        assert.eq(sparse.allowed, true)
    "#,
    )
    .await;
}

#[tokio::test]
async fn a_boolean_context_value_decides_instead_of_aborting_the_check() {
    run_shapes(
        r#"
        local truthy = gated:check(alice, "docs.read", "doc:1", {
          context = { ["app:Flag"] = true },
        })
        assert.eq(truthy.allowed, true)

        local falsy = gated:check(alice, "docs.read", "doc:1", {
          context = { ["app:Flag"] = false },
        })
        assert.eq(falsy.allowed, false)

        local numeric = gated:check(alice, "docs.read", "doc:1", {
          context = { ["app:Flag"] = 12 },
        })
        assert.eq(numeric.allowed, false)
    "#,
    )
    .await;
}

#[tokio::test]
async fn validate_bounds_gates_a_write_the_evaluator_would_refuse() {
    run_shapes(
        r#"
        local ok = eng:validate_bounds({
          { operator = "StringEquals", key = "app:Str", value = "x" },
        })
        assert.not_nil(ok)

        local bad, err = eng:validate_bounds({
          { operator = "DateLessThan", key = "request:Time", value = "2027-01-01T00:00:00" },
        })
        assert.eq(bad, nil)
        assert.contains(err, "timezone")
    "#,
    )
    .await;
}
