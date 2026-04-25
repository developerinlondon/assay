/* Engine console — Config pane.
 *
 * Read-only viewer over the running `engine.toml`. The endpoint
 * (`/api/v1/engine/core/config`) returns the parsed config with
 * `admin_api_keys` redacted to literal "[REDACTED]" so screenshots
 * don't leak credentials.
 */

var AssayEngineConfig = (function () {
  'use strict';

  let ctx = null;

  function render(el, c) {
    ctx = c;
    el.innerHTML =
      '<div class="auth-toolbar">' +
        '<h2 class="section-title">Config</h2>' +
        '<button type="button" class="btn btn-small" id="config-refresh">Refresh</button>' +
      '</div>' +
      '<p class="auth-empty">Read-only view of the running <code>engine.toml</code>. ' +
        'Secrets such as <code>admin_api_keys</code> are redacted.</p>' +
      '<div id="engine-config-wrap"><div class="auth-empty">Loading…</div></div>';
    document.getElementById('config-refresh').addEventListener('click', load);
    load();
  }

  async function load() {
    const wrap = document.getElementById('engine-config-wrap');
    try {
      const cfg = await ctx.api.getConfig();
      const pretty = JSON.stringify(cfg, null, 2);
      wrap.innerHTML = '<pre class="engine-config-pre">' + ctx.escapeHtml(pretty) + '</pre>';
    } catch (err) {
      wrap.innerHTML = '<div class="auth-empty">Error: ' + ctx.escapeHtml(err.message) + '</div>';
    }
  }

  if (typeof window !== 'undefined') {
    window.AssayEngineConfig = { render: render };
  }
  return { render: render };
})();
