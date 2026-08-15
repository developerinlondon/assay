--- @module assay.authz
--- @description In-process authorization engine: policy statements, grants-at-scope with typed bounds, ABAC conditions, deny-wins, asymmetric fail-closed. Decides every agentauthz conformance fixture identically. Pure computation, no I/O, no storage.
--- @category identity
--- @keywords authz, authorization, policy, permissions, abac, rbac, grants, scope, deny-wins, conditions
--- @quickref authz.engine(opts) -> engine | Build an engine from grants, condition keys, scope kinds and an action catalogue
--- @quickref e:check(subjects, action, resource, opts?) -> decision | Decide; read decision.allowed, .decision, .reason
--- @quickref e:validate(statements) -> statements | nil, err | Refuse a statement or condition the engine could never evaluate
--- @quickref e:describe() -> descriptor | The declared vocabulary as data, for an administration surface
--- @quickref e:grants_for(subjects, scope_chain?) -> grants | The grants effective over a chain, for a "why" view

-- The engine is a Rust builtin registered as the `authz` global. This module
-- is the require()-able name for it, so a script pulling it in reads like
-- every other assay module.
return authz
