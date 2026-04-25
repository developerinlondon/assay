/* Cross-console navigation strip controller.
 *
 * Reads `/api/v1/modules` (no auth) so disabled modules' pills don't
 * render. The Engine pill always shows because engine-core is always
 * running. Active console is set by the host (`AssayCrossNav.render({
 * active: 'workflow' | 'auth' | 'engine' })`).
 */

(function () {
  'use strict';

  // The pill registry is intentionally tiny — three known consoles.
  // Each pill knows whether its module must be enabled to render
  // (engine-core is always on; workflow + auth gate on /modules).
  const PILLS = [
    { id: 'workflow', label: 'Workflow', href: '/workflow/', requires: 'workflow' },
    { id: 'auth',     label: 'Auth',     href: '/auth/console',  requires: 'auth' },
    { id: 'engine',   label: 'Engine',   href: '/engine/console', requires: null },
  ];

  function escapeHtml(s) {
    if (s === null || s === undefined) return '';
    return String(s)
      .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;').replace(/'/g, '&#39;');
  }

  async function loadModules() {
    try {
      const r = await fetch('/api/v1/modules', { headers: { 'accept': 'application/json' } });
      if (!r.ok) return [];
      const j = await r.json();
      return Array.isArray(j.modules) ? j.modules : [];
    } catch (_) {
      // Without /modules data we still want the engine pill — fall
      // through with an empty list and let `requires === null` show it.
      return [];
    }
  }

  async function render(opts) {
    opts = opts || {};
    const active = opts.active || '';
    const container = document.getElementById('cross-nav-pills');
    if (!container) return;

    const modules = await loadModules();
    const html = PILLS
      .filter(function (p) { return !p.requires || modules.indexOf(p.requires) !== -1; })
      .map(function (p) {
        const cls = 'cross-nav-pill' + (p.id === active ? ' active' : '');
        return '<a href="' + escapeHtml(p.href) + '" class="' + cls + '"' +
          ' data-pill="' + escapeHtml(p.id) + '">' + escapeHtml(p.label) + '</a>';
      })
      .join('');
    container.innerHTML = html;
  }

  window.AssayCrossNav = { render: render };
})();
