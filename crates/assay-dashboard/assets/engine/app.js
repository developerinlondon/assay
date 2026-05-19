/* Assay Engine Console — main controller.
 *
 * SPA shell mirroring the auth console's shape: each pane is a
 * `{render(el, ctx)}` object that paints into #content. The shared
 * `ctx` carries the API client, the admin token (re-uses the same
 * `assay-admin-token` localStorage key as the auth console so an
 * operator only types it once), and small UI helpers.
 */

(function () {
  'use strict';

  const ADMIN_TOKEN_KEY = 'assay-admin-token';
  let currentView = 'info';
  let adminToken = localStorage.getItem(ADMIN_TOKEN_KEY) || '';
  // Asynchronously set by probeSession(); when true the user has an
  // assay_session cookie that the backend accepts via session+zanzibar.
  let hasSession = false;
  function canCall() { return Boolean(adminToken || hasSession); }

  async function probeSession() {
    try {
      const r = await fetch('/api/v1/engine/auth/whoami', {
        credentials: 'same-origin',
        headers: { 'accept': 'application/json' },
      });
      hasSession = r.ok;
    } catch (_) {
      hasSession = false;
    }
  }

  function escapeHtml(str) {
    if (str === null || str === undefined) return '';
    const d = document.createElement('div');
    d.textContent = String(str);
    return d.innerHTML;
  }

  function formatTime(ts) {
    if (!ts) return '-';
    return new Date(ts * 1000).toLocaleString();
  }

  function formatRelative(ts) {
    if (!ts) return '-';
    const now = Date.now() / 1000;
    const diff = now - ts;
    if (diff < 60) return Math.floor(diff) + 's ago';
    if (diff < 3600) return Math.floor(diff / 60) + 'm ago';
    if (diff < 86400) return Math.floor(diff / 3600) + 'h ago';
    if (diff < 604800) return Math.floor(diff / 86400) + 'd ago';
    return new Date(ts * 1000).toLocaleString();
  }

  function formatDuration(seconds) {
    if (seconds < 60) return Math.floor(seconds) + 's';
    if (seconds < 3600) return Math.floor(seconds / 60) + 'm';
    if (seconds < 86400) return Math.floor(seconds / 3600) + 'h';
    return Math.floor(seconds / 86400) + 'd ' + Math.floor((seconds % 86400) / 3600) + 'h';
  }

  function truncate(s, n) {
    if (!s) return '';
    return s.length > n + 3 ? s.substring(0, n) + '...' : s;
  }

  function toast(msg, kind) {
    const k = kind || 'info';
    let container = document.getElementById('toast-container');
    if (!container) {
      container = document.createElement('div');
      container.id = 'toast-container';
      container.className = 'toast-container';
      document.body.appendChild(container);
    }
    const el = document.createElement('div');
    el.className = 'toast toast-' + k;
    el.textContent = msg;
    container.appendChild(el);
    requestAnimationFrame(function () { el.classList.add('toast-show'); });
    setTimeout(function () {
      el.classList.remove('toast-show');
      setTimeout(function () { el.remove(); }, 200);
    }, 3500);
  }

  function setAdminToken(token) {
    adminToken = token || '';
    if (adminToken) localStorage.setItem(ADMIN_TOKEN_KEY, adminToken);
    else localStorage.removeItem(ADMIN_TOKEN_KEY);
    updateStatusBar();
  }

  function makeCtx() {
    return {
      api: window.AssayEngineApi,
      escapeHtml: escapeHtml,
      formatTime: formatTime,
      formatRelative: formatRelative,
      formatDuration: formatDuration,
      truncate: truncate,
      toast: toast,
      getToken: function () { return adminToken; },
      setToken: setAdminToken,
      switchView: switchView,
      refreshCurrentView: function () { renderCurrentView(); },
    };
  }

  const views = {
    info: typeof AssayEngineInfo !== 'undefined' ? AssayEngineInfo : null,
    modules: typeof AssayEngineModules !== 'undefined' ? AssayEngineModules : null,
    instances: typeof AssayEngineInstances !== 'undefined' ? AssayEngineInstances : null,
    audit: typeof AssayEngineAudit !== 'undefined' ? AssayEngineAudit : null,
    config: typeof AssayEngineConfig !== 'undefined' ? AssayEngineConfig : null,
  };

  function switchView(view) {
    if (!views[view]) return;
    currentView = view;
    document.querySelectorAll('.nav-link[data-view]').forEach(function (link) {
      link.classList.toggle('active', link.dataset.view === view);
    });
    renderCurrentView();
  }

  function renderCurrentView() {
    const component = views[currentView];
    if (!component || !component.render) return;
    // The Info pane is public — every other pane needs the admin
    // token. Render the token banner inline when missing instead of
    // navigating away, so operators always know what to do next.
    if (currentView !== 'info' && !canCall()) {
      renderTokenBanner();
      return;
    }
    component.render(document.getElementById('content'), makeCtx());
  }

  function renderTokenBanner() {
    const el = document.getElementById('content');
    el.innerHTML =
      '<div class="auth-token-banner">' +
        '<strong>Admin token required.</strong> ' +
        'These panes hit <code>/api/v1/engine/core/*</code> endpoints gated by ' +
        '<code>auth.admin_api_keys</code>. Paste a configured token to continue. ' +
        'The token lives in your browser localStorage; clear it on a shared machine.' +
      '</div>' +
      '<div class="auth-form">' +
        '<label for="admin-token-input">Admin token</label>' +
        '<input type="password" id="admin-token-input" placeholder="Bearer token" autocomplete="off" />' +
        '<div class="auth-form-actions">' +
          '<button type="button" class="btn btn-primary" id="admin-token-save">Save token</button>' +
          '<button type="button" class="btn" id="admin-token-clear">Clear</button>' +
        '</div>' +
      '</div>';
    document.getElementById('admin-token-save').addEventListener('click', function () {
      const v = document.getElementById('admin-token-input').value.trim();
      if (!v) { toast('Token cannot be empty', 'error'); return; }
      setAdminToken(v);
      toast('Token saved', 'info');
      switchView(currentView);
    });
    document.getElementById('admin-token-clear').addEventListener('click', function () {
      setAdminToken('');
      toast('Token cleared', 'info');
    });
  }

  function updateStatusBar() {
    const dot = document.getElementById('admin-dot');
    const text = document.getElementById('admin-text');
    if (!dot || !text) return;
    if (adminToken) {
      dot.className = 'status-dot connected';
      text.textContent = 'Admin authenticated (token)';
    } else if (hasSession) {
      dot.className = 'status-dot connected';
      text.textContent = 'Signed in';
    } else {
      dot.className = 'status-dot disconnected';
      text.textContent = 'Not signed in';
    }
  }

  // Populate the status-version footer span by hitting /api/v1/engine/workflow/version
  // (public). Same JS contract as the workflow + auth consoles.
  async function loadVersion() {
    try {
      const r = await fetch('/api/v1/engine/workflow/version', { headers: { 'accept': 'application/json' } });
      if (!r.ok) return;
      const v = await r.json();
      const el = document.getElementById('status-version');
      if (el && v && v.version) {
        el.textContent =
          'v' + v.version + (v.build_profile === 'debug' ? ' (debug)' : '');
      }
    } catch (_) { /* leave placeholder */ }
  }

  // Header-bar identity strip — version, leader dot, instance id.
  // Hits /api/v1/engine/core/info (public, no auth), then mirrors the values
  // into the cross-nav header so operators see them on every console
  // load even before they paste an admin token.
  async function loadHeaderIdentity() {
    try {
      const r = await fetch('/api/v1/engine/core/info', { headers: { 'accept': 'application/json' } });
      if (!r.ok) return;
      const info = await r.json();
      const v = document.getElementById('cross-nav-version');
      if (v && info.version) v.textContent = 'v' + info.version;
      const dot = document.getElementById('cross-nav-leader-dot');
      const txt = document.getElementById('cross-nav-leader-text');
      if (dot) dot.classList.toggle('leader', !!info.leader);
      if (txt) txt.textContent = info.leader ? 'leader' : 'follower';
      const inst = document.getElementById('cross-nav-instance');
      if (inst && info.instance_id) {
        // Truncate UUID to head…tail for the header — the full ID lives
        // on the Info pane.
        const id = info.instance_id;
        inst.textContent = 'instance:' + id.slice(0, 6) + '…' + id.slice(-4);
        inst.title = 'instance ' + id;
      }
    } catch (_) { /* leave placeholders */ }
  }

  function initTheme() {
    // Resolution order:
    //   1. Saved user choice (assay-theme localStorage)
    //   2. ASSAY_WHITELABEL_DEFAULT_THEME from the operator (read off
    //      <html data-default-theme=...>) - lets a brand-pack consumer
    //      pin the SPA to dark or light without per-system overrides.
    //   3. prefers-color-scheme, when default-theme is unset or 'auto'.
    const saved = localStorage.getItem('assay-theme');
    const def = (document.documentElement.dataset.defaultTheme || 'auto');
    const fromBrand = (def === 'dark' || def === 'light') ? def : null;
    const theme = saved
      || fromBrand
      || (window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');
    document.documentElement.setAttribute('data-theme', theme);
  }

  function toggleTheme() {
    const html = document.documentElement;
    const current = html.getAttribute('data-theme');
    const next = current === 'light' ? 'dark' : 'light';
    html.setAttribute('data-theme', next);
    localStorage.setItem('assay-theme', next);
  }

  function init() {
    initTheme();
    Object.keys(views).forEach(function (k) {
      const camel = 'AssayEngine' + k.charAt(0).toUpperCase() + k.slice(1);
      if (typeof window[camel] !== 'undefined') views[k] = window[camel];
    });

    document.getElementById('theme-toggle').addEventListener('click', toggleTheme);
    document.getElementById('sidebar-toggle').addEventListener('click', function () {
      document.getElementById('sidebar').classList.toggle('open');
    });
    document.getElementById('sidebar-collapse').addEventListener('click', function () {
      document.getElementById('sidebar').classList.toggle('collapsed');
    });

    document.querySelector('.sidebar-nav').addEventListener('click', function (e) {
      const link = e.target.closest('.nav-link[data-view]');
      if (!link) return;
      e.preventDefault();
      switchView(link.dataset.view);
      document.getElementById('sidebar').classList.remove('open');
    });

    if (window.AssayCrossNav) {
      window.AssayCrossNav.render({ active: 'engine' });
    }

    updateStatusBar();
    loadVersion();
    loadHeaderIdentity();
    switchView('info');
    // Probe the session cookie asynchronously. If a valid session is
    // present, the SPA can call admin endpoints via session+zanzibar
    // without a token prompt.
    probeSession().then(updateStatusBar);
  }

  window.AssayEngineApp = {
    getToken: function () { return adminToken; },
    setToken: setAdminToken,
    toast: toast,
    escapeHtml: escapeHtml,
    formatTime: formatTime,
    formatRelative: formatRelative,
    formatDuration: formatDuration,
    truncate: truncate,
  };

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
