/* Cross-console navigation strip controller.
 *
 * Reads `/api/v1/engine/core/active-modules` (no auth) so disabled modules' pills don't
 * render. The Engine pill always shows because engine-core is always
 * running. Active console is set by the host (`AssayCrossNav.render({
 * active: 'workflow' | 'auth' | 'engine' })`).
 */

(function () {
  'use strict';

  // The console registry is intentionally tiny — three known consoles.
  // Each entry knows whether its module must be enabled to render
  // (engine-core is always on; workflow + auth gate on /modules).
  // Icons are inline SVGs so they pick up currentColor and don't need a
  // font dependency. Render output goes into #cross-nav-pills (now
  // sitting at the top of the per-console sidebar, not the top strip).
  const CONSOLES = [
    {
      id: 'workflow', label: 'Workflow', href: '/workflow/', requires: 'workflow',
      // gear (settings/process)
      svg: '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>',
    },
    {
      id: 'auth', label: 'Auth', href: '/auth/console', requires: 'auth',
      // shield (auth/security)
      svg: '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/></svg>',
    },
    {
      id: 'engine', label: 'Engine', href: '/engine/console', requires: null,
      // bolt (engine/runtime)
      svg: '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/></svg>',
    },
  ];

  function escapeHtml(s) {
    if (s === null || s === undefined) return '';
    return String(s)
      .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;').replace(/'/g, '&#39;');
  }

  async function loadModules() {
    try {
      const r = await fetch('/api/v1/engine/core/active-modules', { headers: { 'accept': 'application/json' } });
      if (!r.ok) return [];
      const j = await r.json();
      return Array.isArray(j.modules) ? j.modules : [];
    } catch (_) {
      // Without /active-modules data we still want the engine pill — fall
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
    const html = CONSOLES
      .filter(function (c) { return !c.requires || modules.indexOf(c.requires) !== -1; })
      .map(function (c) {
        const cls = 'console-tab' + (c.id === active ? ' active' : '');
        // `title` gives the native browser tooltip on hover; aria-label
        // mirrors it for screen readers since the visible content is
        // icon-only. data-tab keeps an addressable hook for tests.
        return '<a href="' + escapeHtml(c.href) + '" class="' + cls + '"' +
          ' data-tab="' + escapeHtml(c.id) + '"' +
          ' title="' + escapeHtml(c.label) + '"' +
          ' aria-label="' + escapeHtml(c.label) + '">' +
          c.svg +
          '</a>';
      })
      .join('');
    container.innerHTML = html;
  }

  window.AssayCrossNav = { render: render };
})();
