/* Cross-console navigation strip controller.
 *
 * Reads `/api/v1/engine/core/active-modules` (no auth) so disabled modules'
 * pills don't render. The Engine pill always shows because engine-core is
 * always running. Active console is set by the host
 * (`AssayCrossNav.render({ active: 'workflow' | 'auth' | 'vault' | 'engine' })`).
 *
 * Back-to-parent link is rendered alongside the pills when the host's
 * `<nav id="cross-nav-pills">` carries `data-parent-url` and (optionally)
 * `data-parent-name` attributes. The whitelabel renderer substitutes those
 * from `ASSAY_WHITELABEL_PARENT_URL` / `_PARENT_NAME` so every console
 * gets the same back-link without per-template HTML.
 */

(function () {
  'use strict';

  // The console registry — every assay-engine sub-console that has its own
  // SPA. `requires` names the engine module that must be enabled for the
  // pill to render (the modules list comes from /active-modules); engine-
  // core is always on so its `requires` is null. Icons are inline SVGs so
  // they pick up currentColor and don't need a font dependency.
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
      id: 'vault', label: 'Vault', href: '/vault/console', requires: 'vault',
      // key (vault/secrets)
      svg: '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="7.5" cy="15.5" r="4.5"/><path d="m21 2-9.6 9.6"/><path d="m15.5 7.5 3 3L22 7l-3-3"/></svg>',
    },
    {
      id: 'engine', label: 'Engine', href: '/engine/console', requires: null,
      // bolt (engine/runtime)
      svg: '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/></svg>',
    },
  ];

  // Inline-svg arrow used for the back-to-parent link.
  const BACK_ARROW_SVG =
    '<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">' +
    '<path d="M19 12H5"/><path d="m12 19-7-7 7-7"/></svg>';

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

  // Render the back-to-parent link into the host page if the
  // cross-nav-pills container carries data-parent-url. Idempotent: a
  // second call updates the link rather than appending a duplicate.
  function renderBackLink(container) {
    const url = container.getAttribute('data-parent-url');
    if (!url) return;
    const name = container.getAttribute('data-parent-name') || 'Back';
    const target = document.getElementById('cross-nav-back') || (function () {
      const a = document.createElement('a');
      a.id = 'cross-nav-back';
      a.className = 'cross-nav-back';
      // Sit inside the same sidebar-header as cross-nav-pills, before
      // the pills themselves, so tabbing through reaches it first.
      container.parentNode.insertBefore(a, container);
      return a;
    })();
    target.href = url;
    target.title = name;
    target.setAttribute('aria-label', 'Back to ' + name);
    target.innerHTML = BACK_ARROW_SVG +
      '<span class="cross-nav-back-label">' + escapeHtml(name) + '</span>';
  }

  async function render(opts) {
    opts = opts || {};
    const active = opts.active || '';
    const container = document.getElementById('cross-nav-pills');
    if (!container) return;

    renderBackLink(container);

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
