/* Engine console — Info pane.
 *
 * Public read of /api/v1/engine/core/info. Renders six identity cards
 * (version, instance, started, uptime, backend, modules) so an
 * operator can confirm which engine they're talking to without
 * an admin token.
 */

var AssayEngineInfo = (function () {
  'use strict';

  function render(el, ctx) {
    el.innerHTML = '<h2 class="section-title">Engine</h2>' +
      '<div id="engine-info-wrap"><div class="auth-empty">Loading…</div></div>';
    load(ctx);
  }

  async function load(ctx) {
    const wrap = document.getElementById('engine-info-wrap');
    try {
      // info is public — fetch directly so an unauthenticated operator
      // still sees the header bar populated.
      const r = await fetch('/api/v1/engine/core/info', { headers: { 'accept': 'application/json' } });
      if (!r.ok) throw new Error('HTTP ' + r.status);
      const info = await r.json();
      paint(wrap, info, ctx);
    } catch (err) {
      wrap.innerHTML = '<div class="auth-empty">Error: ' + ctx.escapeHtml(err.message) + '</div>';
    }
  }

  function paint(wrap, info, ctx) {
    const now = Date.now() / 1000;
    const uptime = info.started_at ? ctx.formatDuration(now - info.started_at) : '-';
    const backend = info.backend_kind === 'sqlite'
      ? ('sqlite — ' + ctx.escapeHtml(info.backend_data_dir || ''))
      : ('postgres — ' + ctx.escapeHtml(info.backend_url_redacted || ''));
    const modules = (info.modules || []).map(function (m) {
      return '<code>' + ctx.escapeHtml(m) + '</code>';
    }).join(' ') || '<span class="auth-empty">none</span>';

    wrap.innerHTML =
      '<div class="engine-info-grid">' +
        card('Version', 'v' + ctx.escapeHtml(info.version), true) +
        card('Instance ID', ctx.escapeHtml(info.instance_id), true) +
        card('Started', ctx.escapeHtml(ctx.formatTime(info.started_at)), false) +
        card('Uptime', uptime, false) +
        card('Backend', backend, true) +
        card('Modules', modules, false) +
        card('Bind addr', ctx.escapeHtml(info.bind_addr || '-'), true) +
        card('Public URL', ctx.escapeHtml(info.public_url || '-'), true) +
      '</div>' +
      '<p class="auth-empty">' +
        'The Info pane reads from <code>/api/v1/engine/core/info</code> (public, no auth). ' +
        'Other panes require an admin token configured via <code>auth.admin_api_keys</code>.' +
      '</p>';
  }

  function card(label, value, mono) {
    const cls = mono ? 'value' : 'value regular';
    return '<div class="engine-info-card">' +
      '<span class="label">' + label + '</span>' +
      '<span class="' + cls + '">' + value + '</span>' +
    '</div>';
  }

  if (typeof window !== 'undefined') {
    window.AssayEngineInfo = { render: render };
  }
  return { render: render };
})();
