//! Default Zanzibar namespace schema for the vault module.
//!
//! Plan 17 §S4 + §"Corporate hierarchy" — the namespaces that map
//! KV path tree + collections + personal vaults onto the engine's
//! existing assay-auth Zanzibar store.
//!
//! Engine boot calls [`seed_default_namespaces`] after the auth
//! schema migration runs. `define_namespace` is idempotent, so re-
//! seeding on every boot is cheap and keeps the on-disk schema in
//! sync with the running build.
//!
//! ## Namespaces
//!
//! - **vault** — per-user personal vault. Subject types: `user`.
//!   Relations: `owner` (direct user).
//! - **collection** — shared collection. Relations: `admin`, `editor`,
//!   `viewer` (each direct user OR userset like `team:eng#member`).
//! - **kv_path** — server-decryptable KV path tree. Relations: `admin`,
//!   `writer`, `reader`. Permissions cascade — admin > writer > reader.
//! - **team**, **family**, **org** — group containers. Relation:
//!   `member` (direct user OR userset for nested groups).
//!
//! Path-prefix matching for `kv_path` is the existing engine's job
//! (the `assay-auth::zanzibar` recursive-CTE walker handles userset
//! expansion). The namespace just declares the relation shape.

use assay_auth::zanzibar::{NamespaceSchema, RelationDef, TypeRef, ZanzibarStore};

/// Build the default vault-namespace schemas. Returns 6 namespaces:
/// vault, collection, kv_path, team, family, org. Idempotent —
/// callers re-define on every boot.
pub fn default_namespaces() -> Vec<NamespaceSchema> {
    let user = || TypeRef::direct("user");
    let team_member = || TypeRef::userset("team", "member");
    let family_member = || TypeRef::userset("family", "member");
    let org_member = || TypeRef::userset("org", "member");

    let direct_or_groups = || vec![user(), team_member(), family_member(), org_member()];

    vec![
        NamespaceSchema::new("vault")
            .with_relation("owner", RelationDef::relation("owner", vec![user()])),
        NamespaceSchema::new("collection")
            .with_relation("admin", RelationDef::relation("admin", direct_or_groups()))
            .with_relation(
                "editor",
                RelationDef::relation("editor", direct_or_groups()),
            )
            .with_relation(
                "viewer",
                RelationDef::relation("viewer", direct_or_groups()),
            ),
        NamespaceSchema::new("kv_path")
            .with_relation("admin", RelationDef::relation("admin", direct_or_groups()))
            .with_relation(
                "writer",
                RelationDef::relation("writer", direct_or_groups()),
            )
            .with_relation(
                "reader",
                RelationDef::relation("reader", direct_or_groups()),
            ),
        NamespaceSchema::new("team").with_relation(
            "member",
            RelationDef::relation("member", vec![user(), TypeRef::userset("team", "member")]),
        ),
        NamespaceSchema::new("family").with_relation(
            "member",
            RelationDef::relation("member", vec![user(), TypeRef::userset("family", "member")]),
        ),
        NamespaceSchema::new("org")
            .with_relation(
                "member",
                RelationDef::relation("member", vec![user(), TypeRef::userset("org", "member")]),
            )
            .with_relation("admin", RelationDef::relation("admin", vec![user()])),
    ]
}

/// Idempotently seed every default namespace into the supplied
/// ZanzibarStore. Engine boot calls this after the auth schema
/// migration runs.
pub async fn seed_default_namespaces(store: &dyn ZanzibarStore) -> anyhow::Result<()> {
    for schema in default_namespaces() {
        store
            .define_namespace(&schema)
            .await
            .map_err(|e| anyhow::anyhow!("seed vault zanzibar namespace {}: {e}", schema.name))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_namespaces_cover_plan_locked_set() {
        let names: Vec<String> = default_namespaces()
            .iter()
            .map(|s| s.name.clone())
            .collect();
        for expected in ["vault", "collection", "kv_path", "team", "family", "org"] {
            assert!(
                names.contains(&expected.to_string()),
                "missing namespace {expected}; got {names:?}"
            );
        }
    }

    #[test]
    fn collection_has_three_role_relations() {
        let ns = default_namespaces()
            .into_iter()
            .find(|n| n.name == "collection")
            .unwrap();
        for r in ["admin", "editor", "viewer"] {
            assert!(ns.definitions.contains_key(r), "collection missing {r}");
        }
    }

    #[test]
    fn kv_path_admin_writer_reader() {
        let ns = default_namespaces()
            .into_iter()
            .find(|n| n.name == "kv_path")
            .unwrap();
        for r in ["admin", "writer", "reader"] {
            assert!(ns.definitions.contains_key(r), "kv_path missing {r}");
        }
    }
}
