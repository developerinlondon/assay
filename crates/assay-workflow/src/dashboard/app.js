/* Assay Workflow Dashboard - Main App Controller */

(function () {
  'use strict';

  let currentNamespace = 'main';
  let currentView = 'workflows';
  let eventSource = null;

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

  function truncate(str, len) {
    if (!str) return '';
    return str.length > len ? str.substring(0, len) + '...' : str;
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

  // ── Namespace Switcher ─────────────────────────────────

  async function loadNamespaces() {
    const select = document.getElementById('namespace-select');
    try {
      const namespaces = await fetch('/api/v1/namespaces').then((r) => r.json());
      select.innerHTML = namespaces
        .map((ns) => {
          const name = ns.name || ns;
          const sel = name === currentNamespace ? ' selected' : '';
          return '<option value="' + escapeHtml(name) + '"' + sel + '>' + escapeHtml(name) + '</option>';
        })
        .join('');
    } catch (_) {
      select.innerHTML = '<option value="main" selected>main</option>';
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

    eventSource.onmessage = function () {
      refreshCurrentView();
    };

    // Listen for specific event types
    ['workflow_started', 'workflow_completed', 'workflow_failed',
     'workflow_cancelled', 'task_completed', 'task_failed',
     'signal_received', 'schedule_triggered'].forEach(function (evt) {
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

  function renderCurrentView() {
    const component = views[currentView];
    if (component && component.render) {
      component.render(document.getElementById('content'), {
        namespace: currentNamespace,
        apiFetch: apiFetch,
        formatTime: formatTime,
        truncate: truncate,
        isTerminal: isTerminal,
        formatJson: formatJson,
        badgeClass: badgeClass,
        escapeHtml: escapeHtml,
        showDetail: typeof AssayDetail !== 'undefined' ? AssayDetail.showDetail : null,
      });
    }
  }

  function refreshCurrentView() {
    renderCurrentView();
    updateStatusBar();
  }

  // ── Status Bar ─────────────────────────────────────────

  async function updateStatusBar() {
    document.getElementById('status-namespace').textContent = currentNamespace;

    try {
      const workers = await apiFetch('/workers');
      document.getElementById('status-workers').textContent = Array.isArray(workers) ? workers.length : '0';
    } catch (_) {
      document.getElementById('status-workers').textContent = '?';
    }
  }

  // ── Theme Toggle ───────────────────────────────────────

  function initTheme() {
    const saved = localStorage.getItem('assay-theme');
    if (saved) {
      document.documentElement.setAttribute('data-theme', saved);
    }
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

  // ── Initialization ─────────────────────────────────────

  function init() {
    initTheme();

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
      connectSSE();
      refreshCurrentView();
    });

    // Load namespaces then render
    loadNamespaces().then(function () {
      connectSSE();
      switchView('workflows');
      updateStatusBar();
    });
  }

  // Expose globals for components
  window.AssayApp = {
    getNamespace: function () { return currentNamespace; },
    apiFetch: apiFetch,
    formatTime: formatTime,
    truncate: truncate,
    isTerminal: isTerminal,
    formatJson: formatJson,
    badgeClass: badgeClass,
    escapeHtml: escapeHtml,
    refreshCurrentView: refreshCurrentView,
  };

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
