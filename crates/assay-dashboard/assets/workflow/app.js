/* Assay Workflow Dashboard - Main App Controller */

(function () {
  'use strict';

  // Default namespace comes from the URL hash (so F5 restores), then
  // the whitelabel template's data-default-namespace attribute, then
  // "main". Operators running a single-tenant assay-as-a-product point
  // every user at the non-"main" namespace their own runs live in, so
  // nobody has to change the dropdown on first load.
  let currentNamespace =
    parseHash().namespace
    || (document.body && document.body.dataset && document.body.dataset.defaultNamespace)
    || 'main';
  let currentView = 'workflows';
  let eventSource = null;
  // Suppress hash-write side effects when we're applying state read
  // *from* the hash (e.g. on initial load or browser back/forward).
  let suppressHashWrite = false;

  // ── Hash router ────────────────────────────────────────
  //
  // URL hash format: `#ns=<namespace>&wf=<workflow_id>`. Both keys
  // are optional; missing keys mean "no detail open" / "default
  // namespace". We deliberately don't URL-encode the values
  // aggressively — workflow ids and namespace names are URL-safe by
  // design (engine validates).

  function parseHash() {
    var raw = (window.location.hash || '').replace(/^#/, '');
    if (!raw) return {};
    var out = {};
    raw.split('&').forEach(function (pair) {
      var eq = pair.indexOf('=');
      if (eq <= 0) return;
      var k = pair.substring(0, eq);
      var v = decodeURIComponent(pair.substring(eq + 1));
      if (k === 'ns') out.namespace = v;
      else if (k === 'wf') out.wf = v;
    });
    return out;
  }

  function writeHash(state) {
    var parts = [];
    if (state && state.namespace) parts.push('ns=' + encodeURIComponent(state.namespace));
    if (state && state.wf) parts.push('wf=' + encodeURIComponent(state.wf));
    var next = parts.length ? '#' + parts.join('&') : '';
    if (window.location.hash === next) return;
    suppressHashWrite = true;
    if (next) {
      window.location.replace(window.location.pathname + window.location.search + next);
    } else {
      // Strip the hash without leaving a `#` in the URL.
      history.replaceState(null, '', window.location.pathname + window.location.search);
    }
    suppressHashWrite = false;
  }

  // Called by detail.js when it opens or closes a workflow detail
  // panel — keeps the URL hash in sync so F5 / share-link flows can
  // restore the same view.
  function setOpenWorkflow(id) {
    writeHash({ namespace: currentNamespace, wf: id || null });
  }

  // ── Helpers ────────────────────────────────────────────

  function formatTime(ts) {
    if (!ts) return '-';
    const now = Date.now() / 1000;
    const diff = now - ts;
    if (diff < 0) return 'just now';
    if (diff < 60) return Math.floor(diff) + 's ago';
    if (diff < 3600) return Math.floor(diff / 60) + 'm ago';
    if (diff < 86400) return Math.floor(diff / 3600) + 'h ago';
    if (diff < 604800) return Math.floor(diff / 86400) + 'd ago';
    return new Date(ts * 1000).toLocaleDateString();
  }

  // Wall-clock form of a timestamp. Events lists use this so
  // operators can line engine events up against external logs by
  // timestamp. formatTime's relative form is still the default
  // elsewhere (row "created 2m ago" reads more naturally).
  function formatExactTime(ts) {
    if (!ts) return '-';
    const d = new Date(ts * 1000);
    const today = new Date();
    const sameDay = d.toDateString() === today.toDateString();
    const hh = String(d.getHours()).padStart(2, '0');
    const mm = String(d.getMinutes()).padStart(2, '0');
    const ss = String(d.getSeconds()).padStart(2, '0');
    if (sameDay) return hh + ':' + mm + ':' + ss;
    return d.getFullYear() + '-' +
      String(d.getMonth() + 1).padStart(2, '0') + '-' +
      String(d.getDate()).padStart(2, '0') + ' ' +
      hh + ':' + mm + ':' + ss;
  }

  function truncate(str, len) {
    if (!str) return '';
    // The ellipsis itself is 3 chars, so truncating a string that's only
    // a handful of chars over the limit is lossy UI noise — you show
    // "thirty-two-char-id-exactly-thir..." instead of the real 35-char
    // string that would have fit without much column growth. Require at
    // least TRUNCATE_MIN_SAVINGS chars of actual savings to bother.
    var TRUNCATE_MIN_SAVINGS = 4;
    return str.length > len + TRUNCATE_MIN_SAVINGS
      ? str.substring(0, len) + '...'
      : str;
  }

  function isTerminal(status) {
    return ['COMPLETED', 'FAILED', 'CANCELLED', 'TERMINATED'].includes(
      (status || '').toUpperCase()
    );
  }

  function formatJson(str) {
    if (!str) return '';
    try {
      return JSON.stringify(JSON.parse(str), null, 2);
    } catch (_) {
      return str;
    }
  }

  function badgeClass(status) {
    const s = (status || '').toUpperCase();
    const map = {
      PENDING: 'badge-pending',
      RUNNING: 'badge-running',
      COMPLETED: 'badge-completed',
      FAILED: 'badge-failed',
      WAITING: 'badge-waiting',
      CANCELLED: 'badge-cancelled',
      TERMINATED: 'badge-cancelled',
    };
    return map[s] || 'badge-pending';
  }

  function escapeHtml(str) {
    if (!str) return '';
    const d = document.createElement('div');
    d.textContent = str;
    return d.innerHTML;
  }

  async function apiFetch(path, opts) {
    const sep = path.includes('?') ? '&' : '?';
    const url = '/api/v1' + path + sep + 'namespace=' + encodeURIComponent(currentNamespace);
    const res = await fetch(url, opts);
    if (!res.ok) {
      const body = await res.text();
      throw new Error(body || res.statusText);
    }
    if (res.status === 204 || res.headers.get('content-length') === '0') return null;
    return res.json();
  }

  /// Fetch without auto-injecting the namespace query param — used for
  /// endpoints that don't take one (e.g. /version, /namespaces).
  async function apiFetchRaw(path, opts) {
    const res = await fetch('/api/v1' + path, opts);
    if (!res.ok) {
      const body = await res.text();
      throw new Error(body || res.statusText);
    }
    if (res.status === 204 || res.headers.get('content-length') === '0') return null;
    return res.json();
  }

  /// Transient toast at the bottom-right for success/error feedback on
  /// mutations. Auto-dismisses after 3 seconds. No hard dep — if the
  /// DOM node isn't there yet (first render), it's created lazily.
  function toast(message, kind) {
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
    el.textContent = message;
    container.appendChild(el);
    // Trigger enter animation on next frame
    requestAnimationFrame(function () { el.classList.add('toast-show'); });
    setTimeout(function () {
      el.classList.remove('toast-show');
      setTimeout(function () { el.remove(); }, 200);
    }, 3000);
  }

  // ── Namespace Switcher ─────────────────────────────────

  async function loadNamespaces() {
    const select = document.getElementById('namespace-select');
    const statusSelect = document.getElementById('status-namespace-select');
    try {
      const namespaces = await fetch('/api/v1/namespaces').then((r) => r.json());
      const options = namespaces
        .map((ns) => {
          const name = ns.name || ns;
          const sel = name === currentNamespace ? ' selected' : '';
          return '<option value="' + escapeHtml(name) + '"' + sel + '>' + escapeHtml(name) + '</option>';
        })
        .join('');
      select.innerHTML = options;
      if (statusSelect) statusSelect.innerHTML = options;
    } catch (_) {
      select.innerHTML = '<option value="main" selected>main</option>';
      if (statusSelect) statusSelect.innerHTML = '<option value="main" selected>main</option>';
    }
    // Re-enhance to a styled custom dropdown — re-runs cleanly even
    // after prior enhancement (rebuilds options in place).
    if (typeof AssaySelect !== 'undefined') {
      AssaySelect.enhance(select, { className: 'assay-select-sidebar' });
      if (statusSelect) AssaySelect.enhance(statusSelect, { className: 'assay-select-statusbar' });
    }
  }

  // ── SSE Connection ─────────────────────────────────────

  function connectSSE() {
    if (eventSource) {
      eventSource.close();
    }

    const url = '/api/v1/events/stream?namespace=' + encodeURIComponent(currentNamespace);
    eventSource = new EventSource(url);

    const dot = document.getElementById('connection-dot');
    const text = document.getElementById('connection-text');

    eventSource.onopen = function () {
      dot.className = 'status-dot connected';
      text.textContent = 'Connected';
    };

    eventSource.onerror = function () {
      dot.className = 'status-dot disconnected';
      text.textContent = 'Disconnected';
      eventSource.close();
      setTimeout(connectSSE, 5000);
    };

    // `onmessage` fires for every SSE frame. We deliberately do NOT
    // hook a blanket refresh here — most of our events are semantic
    // (workflow_*, schedule_*) and listed below, and triggering a
    // full table re-render on signal_received would blow away any
    // inline-expanded detail panel the operator is actively using.

    // Refresh the list only on events that actually change what the
    // workflow row displays. `signal_received` is intentionally
    // excluded — a signal arriving doesn't change engine status, and
    // re-rendering the table would destroy inline row expansions
    // (folding the operator's active Pipeline tab mid-interaction).
    // The Pipeline tab has its own 1Hz poller that picks up app-level
    // changes from register_query snapshots without touching the
    // table markup.
    ['workflow_started', 'workflow_running', 'workflow_completed',
     'workflow_failed', 'workflow_cancelled', 'workflow_terminated',
     'task_completed', 'task_failed', 'schedule_triggered'].forEach(function (evt) {
      eventSource.addEventListener(evt, function () {
        refreshCurrentView();
      });
    });
  }

  // ── View Switching ─────────────────────────────────────

  const views = {
    workflows: typeof AssayWorkflows !== 'undefined' ? AssayWorkflows : null,
    schedules: typeof AssaySchedules !== 'undefined' ? AssaySchedules : null,
    workers: typeof AssayWorkers !== 'undefined' ? AssayWorkers : null,
    queues: typeof AssayQueues !== 'undefined' ? AssayQueues : null,
    settings: typeof AssaySettings !== 'undefined' ? AssaySettings : null,
  };

  function switchView(view) {
    if (!views[view]) return;

    currentView = view;

    // Update nav active state
    document.querySelectorAll('.nav-link[data-view]').forEach(function (link) {
      link.classList.toggle('active', link.dataset.view === view);
    });

    // Close detail panel if open
    if (typeof AssayDetail !== 'undefined') {
      AssayDetail.closeDetail();
    }

    renderCurrentView();
  }

  // Cross-navigate from one detail panel to another — e.g. from the
  // Recent claimed runs table on a worker's detail to the workflow
  // itself. Switches sidebar nav to the right view then pre-expands
  // the target row via each component's setPendingExpand hook. Fire
  // and forget — if the view can't find the row (e.g. the target
  // isn't on the current page), the row expansion silently no-ops.
  function navigate(kind, id) {
    if (!kind || !id) return;
    if (kind === 'workflow') {
      if (window.AssayWorkflows && window.AssayWorkflows.setExpandedId) {
        window.AssayWorkflows.setExpandedId(id);
      }
      switchView('workflows');
      // Also open the side-panel detail so the workflow shows even
      // if it's not on the current list page (claimed_by queries can
      // point at old workflows that have paged out).
      if (typeof AssayDetail !== 'undefined' && AssayDetail.showDetail) {
        AssayDetail.showDetail(id, makeCtx());
      }
    } else if (kind === 'worker') {
      if (window.AssayWorkers && window.AssayWorkers.setPendingExpand) {
        window.AssayWorkers.setPendingExpand(id);
      }
      switchView('workers');
    } else if (kind === 'queue') {
      if (window.AssayQueues && window.AssayQueues.setPendingExpand) {
        window.AssayQueues.setPendingExpand(id);
      }
      switchView('queues');
    } else if (kind === 'workflow_type') {
      // Filter the Workflows list to runs of a specific type (e.g.
      // clicking the "DemoPipeline" chip on a worker's detail).
      // Routes through the existing search box so the filter shows
      // up visibly + the operator can clear it from the UI.
      if (window.AssayWorkflows && window.AssayWorkflows.setSearchTerm) {
        window.AssayWorkflows.setSearchTerm(id);
      }
      switchView('workflows');
    }
  }

  // Shared context bag passed to every view + the modal-driven
  // action handlers. Single source of truth so detail.js, workflows.js,
  // and AssayActions all see the same helpers.
  function makeCtx() {
    return {
      namespace: currentNamespace,
      getNamespace: function () { return currentNamespace; },
      apiFetch: apiFetch,
      apiFetchRaw: apiFetchRaw,
      toast: toast,
      formatTime: formatTime,
      formatExactTime: formatExactTime,
      truncate: truncate,
      isTerminal: isTerminal,
      formatJson: formatJson,
      badgeClass: badgeClass,
      escapeHtml: escapeHtml,
      refreshCurrentView: refreshCurrentView,
      navigate: navigate,
      showDetail: typeof AssayDetail !== 'undefined' ? AssayDetail.showDetail : null,
      actions: typeof AssayActions !== 'undefined' ? AssayActions : null,
    };
  }

  function renderCurrentView() {
    const component = views[currentView];
    if (component && component.render) {
      component.render(document.getElementById('content'), makeCtx());
    }
  }

  function refreshCurrentView() {
    renderCurrentView();
    updateStatusBar();
  }

  // ── Status Bar ─────────────────────────────────────────

  async function updateStatusBar() {
    document.getElementById('status-namespace').textContent = currentNamespace;
    // Keep the status-bar select in sync with the current namespace —
    // matters when the switch happened through the sidebar select or
    // programmatically.
    var statusSel = document.getElementById('status-namespace-select');
    if (statusSel && statusSel.value !== currentNamespace) {
      statusSel.value = currentNamespace;
    }

    try {
      const workers = await apiFetch('/workers');
      document.getElementById('status-workers').textContent = Array.isArray(workers) ? workers.length : '0';
    } catch (_) {
      document.getElementById('status-workers').textContent = '?';
    }
  }

  // ── Theme Toggle ───────────────────────────────────────

  function initTheme() {
    // Mirror the assay.rs site: explicit user choice overrides OS preference.
    // If no saved choice, follow `prefers-color-scheme`.
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

  // ── Mobile Sidebar ─────────────────────────────────────

  function toggleSidebar() {
    document.getElementById('sidebar').classList.toggle('open');
  }

  // ── Desktop Sidebar Collapse ──────────────────────────

  function initSidebarCollapsed() {
    if (localStorage.getItem('assay-sidebar-collapsed') === '1') {
      document.getElementById('sidebar').classList.add('collapsed');
    }
  }

  function toggleSidebarCollapsed() {
    const sidebar = document.getElementById('sidebar');
    const isNow = sidebar.classList.toggle('collapsed');
    localStorage.setItem('assay-sidebar-collapsed', isNow ? '1' : '0');
  }

  // ── Initialization ─────────────────────────────────────

  function init() {
    initTheme();
    initSidebarCollapsed();

    // Re-resolve view references after all scripts load
    views.workflows = typeof AssayWorkflows !== 'undefined' ? AssayWorkflows : null;
    views.schedules = typeof AssaySchedules !== 'undefined' ? AssaySchedules : null;
    views.workers = typeof AssayWorkers !== 'undefined' ? AssayWorkers : null;
    views.queues = typeof AssayQueues !== 'undefined' ? AssayQueues : null;
    views.settings = typeof AssaySettings !== 'undefined' ? AssaySettings : null;

    // Theme toggle
    document.getElementById('theme-toggle').addEventListener('click', toggleTheme);

    // Mobile sidebar
    document.getElementById('sidebar-toggle').addEventListener('click', toggleSidebar);

    // Desktop sidebar collapse
    document.getElementById('sidebar-collapse').addEventListener('click', toggleSidebarCollapsed);

    // Nav links (event delegation on sidebar)
    document.querySelector('.sidebar-nav').addEventListener('click', function (e) {
      const link = e.target.closest('.nav-link[data-view]');
      if (!link) return;
      e.preventDefault();
      switchView(link.dataset.view);
      // Close mobile sidebar
      document.getElementById('sidebar').classList.remove('open');
    });

    // Namespace change
    document.getElementById('namespace-select').addEventListener('change', function (e) {
      currentNamespace = e.target.value;
      writeHash({ namespace: currentNamespace });
      connectSSE();
      refreshCurrentView();
    });

    // Status-bar namespace switcher change handler — same behaviour as
    // the sidebar select (sync currentNamespace, reconnect SSE, refresh
    // view). No click handler needed: the native <select> handles its
    // own dropdown, anchored at the status-bar location.
    var statusSelectEl = document.getElementById('status-namespace-select');
    if (statusSelectEl) {
      statusSelectEl.addEventListener('change', function (e) {
        currentNamespace = e.target.value;
        writeHash({ namespace: currentNamespace });
        // Keep the sidebar select in sync.
        var sidebar = document.getElementById('namespace-select');
        if (sidebar && sidebar.value !== currentNamespace) {
          sidebar.value = currentNamespace;
        }
        connectSSE();
        refreshCurrentView();
      });
    }

    // Browser back/forward — re-apply the hash. Skip our own writes
    // (suppressHashWrite is set when we update the URL ourselves).
    window.addEventListener('hashchange', function () {
      if (suppressHashWrite) return;
      var h = parseHash();
      if (h.namespace && h.namespace !== currentNamespace) {
        currentNamespace = h.namespace;
        var sb = document.getElementById('namespace-select');
        if (sb) sb.value = currentNamespace;
        var ssb = document.getElementById('status-namespace-select');
        if (ssb) ssb.value = currentNamespace;
        connectSSE();
        refreshCurrentView();
      }
      if (h.wf) {
        if (typeof AssayDetail !== 'undefined') AssayDetail.showDetail(h.wf, makeCtx());
      } else if (typeof AssayDetail !== 'undefined') {
        AssayDetail.closeDetail();
      }
    });

    // Wire the modal-driven action handlers (Signal/Cancel/Terminate
    // /Continue-as-new). Workflows.js renders the row icons that
    // call AssayActions; the actions themselves own the modal flows.
    if (typeof AssayActions !== 'undefined') {
      AssayActions.init(makeCtx());
    }

    // Snapshot the hash before any switchView() call — switchView
    // fires closeDetail which clears the hash's `wf` key, so we need
    // the initial value captured first to restore state on F5.
    var initialHash = parseHash();

    loadNamespaces().then(function () {
      connectSSE();
      switchView('workflows');
      updateStatusBar();
      if (initialHash.wf && typeof AssayDetail !== 'undefined') {
        AssayDetail.showDetail(initialHash.wf, makeCtx());
      }
    });

    // Stamp engine version in the status bar so operators know which
    // build they're talking to. Fire-and-forget: if /version doesn't
    // exist (older engine), the placeholder stays.
    loadVersion();
  }

  async function loadVersion() {
    try {
      const v = await apiFetchRaw('/version');
      if (v && v.version) {
        const el = document.getElementById('status-version');
        if (el) {
          el.textContent =
            'v' + v.version + (v.build_profile === 'debug' ? ' (debug)' : '');
        }
      }
    } catch (_) {
      // Leave the placeholder — not worth surfacing as an error.
    }
  }

  // Expose globals for components
  window.AssayApp = {
    getNamespace: function () { return currentNamespace; },
    apiFetch: apiFetch,
    apiFetchRaw: apiFetchRaw,
    toast: toast,
    formatTime: formatTime,
    truncate: truncate,
    isTerminal: isTerminal,
    formatJson: formatJson,
    badgeClass: badgeClass,
    escapeHtml: escapeHtml,
    refreshCurrentView: refreshCurrentView,
    setOpenWorkflow: setOpenWorkflow,
  };

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
