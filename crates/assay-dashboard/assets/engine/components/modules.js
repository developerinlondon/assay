/* Engine console — Modules pane.
 *
 * Lists every row from `engine.modules` with a toggle button. Toggle
 * marks the row enabled / disabled in the DB but the running engine
 * doesn't pick it up until restart — surface a banner explaining that.
 */

var AssayEngineModules = (function () {
  'use strict';

  let ctx = null;

  function render(el, c) {
    ctx = c;
    el.innerHTML =
      '<div class="auth-toolbar"><h2 class="section-title">Modules</h2></div>' +
      '<div class="engine-restart-banner">' +
        'Toggling a module flips the row in <code>engine.modules</code>; the running ' +
        'engine loads/unloads the module on restart.' +
      '</div>' +
      '<div id="engine-modules-wrap"><div class="auth-empty">Loading…</div></div>';

    el.addEventListener('click', function (e) {
      const btn = e.target.closest('button[data-toggle]');
      if (!btn) return;
      const name = btn.dataset.toggle;
      const target = btn.dataset.next === 'true';
      doToggle(name, target);
    });

    load();
  }

  async function load() {
    const wrap = document.getElementById('engine-modules-wrap');
    try {
      const data = await ctx.api.listModules();
      paint(wrap, data.items || []);
    } catch (err) {
      wrap.innerHTML = '<div class="auth-empty">Error: ' + ctx.escapeHtml(err.message) + '</div>';
    }
  }

  function paint(wrap, items) {
    if (!items.length) {
      wrap.innerHTML = '<div class="auth-empty">No modules in <code>engine.modules</code>.</div>';
      return;
    }
    let html = '<table class="data-table"><thead><tr>' +
      '<th>Name</th><th>Enabled</th><th>Version</th><th>Enabled at</th><th>Enabled by</th><th></th>' +
      '</tr></thead><tbody>';
    for (let i = 0; i < items.length; i++) {
      const m = items[i];
      const next = m.enabled ? 'false' : 'true';
      const nextLabel = m.enabled ? 'Disable' : 'Enable';
      const nextClass = m.enabled ? 'btn btn-small btn-danger' : 'btn btn-small btn-primary';
      html += '<tr>' +
        '<td><code>' + ctx.escapeHtml(m.name) + '</code></td>' +
        '<td>' + (m.enabled ? '<strong>enabled</strong>' : 'disabled') + '</td>' +
        '<td>' + ctx.escapeHtml(m.version || '-') + '</td>' +
        '<td>' + (m.enabled_at ? ctx.formatTime(m.enabled_at) : '-') + '</td>' +
        '<td>' + ctx.escapeHtml(m.enabled_by || '-') + '</td>' +
        '<td><button class="' + nextClass + '" data-toggle="' + ctx.escapeHtml(m.name) +
          '" data-next="' + next + '">' + nextLabel + '</button></td>' +
      '</tr>';
    }
    html += '</tbody></table>';
    wrap.innerHTML = html;
  }

  async function doToggle(name, target) {
    if (!confirm((target ? 'Enable' : 'Disable') + ' module ' + name +
      '?\n\nRestart the engine for the change to take effect.')) return;
    try {
      const r = await ctx.api.toggleModule(name, target);
      ctx.toast(r.message || 'Module flag updated', 'info');
      load();
    } catch (err) {
      ctx.toast('Toggle failed: ' + err.message, 'error');
    }
  }

  if (typeof window !== 'undefined') {
    window.AssayEngineModules = { render: render };
  }
  return { render: render };
})();
