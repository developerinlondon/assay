/* Assay Auth Console — main controller (phase 8b)
 *
 * SPA shell that mirrors workflow's app.js shape: sidebar nav switches
 * the active view, each view module is a {render(el, ctx)} object that
 * paints itself into the main content area. The shared `ctx` carries
 * the API client, the admin token, and small UI helpers.
 */

(function () {
  'use strict';

  const ADMIN_TOKEN_KEY = 'assay-admin-token';
  let currentView = 'users';
  let adminToken = localStorage.getItem(ADMIN_TOKEN_KEY) || '';

  // Tiny escapeHtml helper — the auth console fields are mostly
  // operator-controlled but admin-supplied display_names + emails can
  // contain user-controlled content, so always escape before render.
  function escapeHtml(str) {
    if (str === null || str === undefined) return '';
    const d = document.createElement('div');
    d.textContent = String(str);
    return d.innerHTML;
  }

  function formatTime(ts) {
    if (!ts) return '-';
    const now = Date.now() / 1000;
    const diff = now - ts;
    if (diff < 60) return Math.floor(diff) + 's ago';
    if (diff < 3600) return Math.floor(diff / 60) + 'm ago';
    if (diff < 86400) return Math.floor(diff / 3600) + 'h ago';
    if (diff < 604800) return Math.floor(diff / 86400) + 'd ago';
    return new Date(ts * 1000).toLocaleString();
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
    if (adminToken) {
      localStorage.setItem(ADMIN_TOKEN_KEY, adminToken);
    } else {
      localStorage.removeItem(ADMIN_TOKEN_KEY);
    }
    updateStatusBar();
  }

  function makeCtx() {
    return {
      api: window.AssayAuthApi,
      escapeHtml: escapeHtml,
      formatTime: formatTime,
      truncate: truncate,
      toast: toast,
      getToken: function () { return adminToken; },
      setToken: setAdminToken,
      switchView: switchView,
      refreshCurrentView: function () { renderCurrentView(); },
    };
  }

  const views = {
    users: typeof AssayAuthUsers !== 'undefined' ? AssayAuthUsers : null,
    sessions: typeof AssayAuthSessions !== 'undefined' ? AssayAuthSessions : null,
    'oidc-clients': typeof AssayAuthOidcClients !== 'undefined' ? AssayAuthOidcClients : null,
    'oidc-upstream': typeof AssayAuthOidcUpstream !== 'undefined' ? AssayAuthOidcUpstream : null,
    zanzibar: typeof AssayAuthZanzibar !== 'undefined' ? AssayAuthZanzibar : null,
    keys: typeof AssayAuthKeys !== 'undefined' ? AssayAuthKeys : null,
    audit: typeof AssayAuthAudit !== 'undefined' ? AssayAuthAudit : null,
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
    component.render(document.getElementById('content'), makeCtx());
  }

  function renderTokenBanner() {
    const el = document.getElementById('content');
    el.innerHTML =
      '<div class="auth-token-banner">' +
        '<strong>Admin token required.</strong> ' +
        'These endpoints are gated by <code>auth.admin_api_keys</code>. ' +
        'Paste a configured token to continue. The token is stored in your browser ' +
        'localStorage; clear it on a shared machine.' +
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
    if (adminToken) {
      dot.className = 'status-dot connected';
      text.textContent = 'Admin authenticated';
    } else {
      dot.className = 'status-dot disconnected';
      text.textContent = 'No admin token';
    }
  }

  function initTheme() {
    const saved = localStorage.getItem('assay-theme');
    const theme = saved
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

  // Populate the "Powered by Assay Engine vX.Y.Z" footer span by hitting
  // /api/v1/version (no auth — public). Keeps the engine + auth consoles
  // showing the same version their users are actually running against.
  async function loadVersion() {
    try {
      const r = await fetch('/api/v1/version', { headers: { 'accept': 'application/json' } });
      if (!r.ok) return;
      const v = await r.json();
      const el = document.getElementById('status-version');
      if (el && v && v.version) {
        el.textContent =
          'v' + v.version + (v.build_profile === 'debug' ? ' (debug)' : '');
      }
    } catch (_) {
      // Leave the placeholder — not worth surfacing as an error.
    }
  }

  function init() {
    initTheme();

    // Re-resolve view references after all scripts load
    Object.keys(views).forEach(function (k) {
      const camel = 'AssayAuth' + k.split('-').map(function (s) {
        return s.charAt(0).toUpperCase() + s.slice(1);
      }).join('');
      if (typeof window[camel] !== 'undefined') {
        views[k] = window[camel];
      }
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
      if (!adminToken) {
        toast('Set an admin token first', 'error');
        return;
      }
      switchView(link.dataset.view);
      document.getElementById('sidebar').classList.remove('open');
    });

    if (window.AssayCrossNav) {
      window.AssayCrossNav.render({ active: 'auth' });
    }
    updateStatusBar();
    loadVersion();
    loadHeaderIdentity();
    if (adminToken) {
      switchView('users');
    } else {
      renderTokenBanner();
    }
  }

  // Populate the cross-nav header bar identity strip from the public
  // /api/v1/engine/info endpoint. Same shape as the workflow + engine
  // shells; duplicated here so the auth shell doesn't depend on either
  // sibling SPA's app.js.
  async function loadHeaderIdentity() {
    try {
      const r = await fetch('/api/v1/engine/info', { headers: { 'accept': 'application/json' } });
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
        const id = info.instance_id;
        inst.textContent = 'instance:' + id.slice(0, 6) + '…' + id.slice(-4);
        inst.title = 'instance ' + id;
      }
    } catch (_) { /* leave placeholders */ }
  }

  window.AssayAuthApp = {
    getToken: function () { return adminToken; },
    setToken: setAdminToken,
    toast: toast,
    escapeHtml: escapeHtml,
    formatTime: formatTime,
    truncate: truncate,
  };

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
