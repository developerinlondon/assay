use std::sync::Arc;

use crate::whitelabel::WhitelabelConfig;

/// Axum state for the dashboard router.
///
/// Holds the branding/whitelabel config and the per-process asset
/// version stamp. Stored behind `Arc<DashboardCtx>` so it can be
/// cheaply cloned across handler calls.
#[derive(Clone)]
pub struct DashboardCtx {
    pub whitelabel: Arc<WhitelabelConfig>,
    /// Per-process asset version stamp. Embedded into served HTML so
    /// every engine restart produces unique asset URLs, breaking both
    /// browser and CDN caches automatically after a redeploy.
    pub asset_version: Arc<String>,
}

impl DashboardCtx {
    /// Construct from an already-computed version string and a
    /// whitelabel config.
    pub fn new(whitelabel: Arc<WhitelabelConfig>, asset_version: String) -> Self {
        Self {
            whitelabel,
            asset_version: Arc::new(asset_version),
        }
    }
}
