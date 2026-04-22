/* Assay Workflow Dashboard — Workers view (v0.12.0)
 *
 * List view + click-through detail. Row click expands inline (same
 * pattern as workflows) showing the full worker record: heartbeat
 * age, registered workflow types / activities, concurrency limits,
 * current namespace + queue, and the last handful of workflows it
 * claimed. Gives operators a one-stop diagnosis surface without
 * pulling logs.
 */

var AssayWorkers = (function () {
  'use strict';

  let ctx = null;
  let container = null;
  let refreshTimer = null;
  let expandedWorkerId = null;

  function render(el, context) {
    ctx = context;
    container = el;

    if (refreshTimer) clearInterval(refreshTimer);

    el.innerHTML =
      '<div class="toolbar">' +
        '<h2 class="section-title">Workers</h2>' +
      '</div>' +
      '<div id="workers-table-wrap"></div>';

    loadWorkers();

    refreshTimer = setInterval(loadWorkers, 10000);

    var observer = new MutationObserver(function () {
      if (!document.body.contains(el)) {
        clearInterval(refreshTimer);
        refreshTimer = null;
        observer.disconnect();
      }
    });
    observer.observe(document.body, { childList: true, subtree: true });

    el.addEventListener('click', function (e) {
      // Cross-link — nav to another view, don't toggle expansion.
      var cross = e.target.closest('.cross-link[data-nav]');
      if (cross) {
        e.preventDefault();
        e.stopPropagation();
        if (ctx.navigate) ctx.navigate(cross.dataset.nav, cross.dataset.id);
        return;
      }
      var row = e.target.closest('tr[data-worker-id]');
      if (!row) return;
      e.preventDefault();
      toggleRowDetail(row);
    });
  }

  async function loadWorkers() {
    var wrap = container.querySelector('#workers-table-wrap');
    if (!wrap) return;

    try {
      var workers = await ctx.apiFetch('/workers');
      renderTable(wrap, workers || []);
      // Re-open the previously expanded row if it's still on this page.
      if (expandedWorkerId) {
        var row = wrap.querySelector('tr[data-worker-id="' + cssEscape(expandedWorkerId) + '"]');
        if (row) toggleRowDetail(row);
        else expandedWorkerId = null;
      }
    } catch (err) {
      wrap.innerHTML = '<div class="empty-state"><p>Error: ' + ctx.escapeHtml(err.message) + '</p></div>';
    }
  }

  function renderTable(wrap, workers) {
    if (workers.length === 0) {
      wrap.innerHTML = '<div class="empty-state"><p>No workers registered</p></div>';
      return;
    }

    var now = Date.now() / 1000;

    var html =
      '<table class="data-table"><thead><tr>' +
        '<th>Status</th>' +
        '<th>ID</th>' +
        '<th>Identity</th>' +
        '<th>Namespace</th>' +
        '<th>Queue</th>' +
        '<th>Active Tasks</th>' +
        '<th>Last Heartbeat</th>' +
      '</tr></thead><tbody>';

    for (var i = 0; i < workers.length; i++) {
      var w = workers[i];
      var hbAge = now - (w.last_heartbeat || 0);
      var dotClass = hbAge < 30 ? 'healthy' : hbAge < 60 ? 'warning' : 'stale';
      var maxTasks = w.max_concurrent_workflows || w.max_concurrent_activities || '-';

      html +=
        '<tr class="clickable-row" data-worker-id="' + ctx.escapeHtml(w.id) + '">' +
          '<td><span class="worker-dot ' + dotClass + '" title="' +
            (dotClass === 'healthy' ? 'Healthy' : dotClass === 'warning' ? 'Slow heartbeat' : 'Stale') +
          '"></span></td>' +
          '<td class="mono clickable" title="' + ctx.escapeHtml(w.id) + '">' +
            ctx.escapeHtml(ctx.truncate(w.id, 24)) + '</td>' +
          '<td>' + ctx.escapeHtml(w.identity || '-') + '</td>' +
          '<td>' + ctx.escapeHtml(w.namespace || '-') + '</td>' +
          '<td class="mono">' + ctx.escapeHtml(w.task_queue || '-') + '</td>' +
          '<td>' + (w.active_tasks || 0) + '/' + maxTasks + '</td>' +
          '<td>' + ctx.formatTime(w.last_heartbeat) + '</td>' +
        '</tr>';
    }

    html += '</tbody></table>';
    wrap.innerHTML = html;
  }

  // ── Inline row expansion ────────────────────────────────────────
  function toggleRowDetail(row) {
    var workerId = row.dataset.workerId;
    var next = row.nextElementSibling;
    if (next && next.classList.contains('worker-detail-row') &&
        next.dataset.forId === workerId) {
      next.remove();
      row.classList.remove('wf-row-expanded');
      expandedWorkerId = null;
      return;
    }
    var openDetails = row.parentNode.querySelectorAll('.worker-detail-row');
    for (var i = 0; i < openDetails.length; i++) openDetails[i].remove();
    var openParents = row.parentNode.querySelectorAll('.wf-row-expanded');
    for (var j = 0; j < openParents.length; j++) {
      openParents[j].classList.remove('wf-row-expanded');
    }
    var colCount = row.children.length;
    var detailRow = document.createElement('tr');
    detailRow.className = 'worker-detail-row wf-detail-row';
    detailRow.dataset.forId = workerId;
    detailRow.innerHTML = '<td colspan="' + colCount + '">' +
      '<div class="wf-inline-detail"><p class="detail-muted">Loading worker detail…</p></div>' +
    '</td>';
    row.parentNode.insertBefore(detailRow, row.nextSibling);
    row.classList.add('wf-row-expanded');
    expandedWorkerId = workerId;
    populateDetail(detailRow.querySelector('.wf-inline-detail'), workerId);
  }

  async function populateDetail(target, workerId) {
    try {
      var workers = await ctx.apiFetch('/workers');
      var worker = Array.isArray(workers) ? workers.find(function (w) { return w.id === workerId; }) : null;
      if (!worker) throw new Error('worker ' + workerId + ' not found');
      var claimed = await ctx.apiFetch(
        '/workflows?claimed_by=' + encodeURIComponent(workerId) + '&limit=20'
      ).catch(function () { return []; });
      target.innerHTML = renderDetailHtml(worker, claimed || []);
    } catch (err) {
      target.innerHTML = '<div class="error-box">' + ctx.escapeHtml(err.message) + '</div>';
    }
  }

  function renderDetailHtml(w, claimed) {
    var workflows = safeParseList(w.workflows);
    var activities = safeParseList(w.activities);
    // Identity-card fields that ARE on the row (status, id, identity,
    // namespace, queue, active tasks, heartbeat) are intentionally
    // dropped from the expansion — duplication just adds vertical
    // noise. Only the row-missing secondary fields (registered-at +
    // concurrency limits) go into a slim meta strip at the top.
    var metaStrip =
      '<div class="row-detail-meta">' +
        '<span><span class="row-detail-meta-label">Registered</span> ' +
          ctx.formatTime(w.registered_at) + ' (' +
          ctx.formatExactTime(w.registered_at) + ')</span>' +
        '<span class="row-detail-meta-sep">&middot;</span>' +
        '<span><span class="row-detail-meta-label">Concurrency</span> ' +
          'wf <strong>' + (w.max_concurrent_workflows != null ? w.max_concurrent_workflows : '-') + '</strong>' +
          ' &middot; act <strong>' + (w.max_concurrent_activities != null ? w.max_concurrent_activities : '-') + '</strong>' +
        '</span>' +
      '</div>';

    // Two columns of what the row CAN'T carry: registered workflow
    // handlers + registered activity handlers. Chip lists wrap
    // naturally inside their grid cells.
    var leftCol = '<h4 class="detail-subhead">Registered workflows (' + workflows.length + ')</h4>';
    leftCol += workflows.length
      ? '<ul class="chip-list">' + workflows.map(function (t) {
          var n = ctx.escapeHtml(t);
          // Chip is a clickable cross-link: filters the Workflows
          // list to runs of this type via ctx.navigate('workflow_type').
          return '<li class="chip chip-link mono">' +
            '<a href="#" class="cross-link" data-nav="workflow_type" data-id="' + n + '">' + n + '</a>' +
          '</li>';
        }).join('') + '</ul>'
      : '<p class="detail-muted">None.</p>';

    var rightCol = '<h4 class="detail-subhead">Registered activities (' + activities.length + ')</h4>';
    rightCol += activities.length
      ? '<ul class="chip-list">' + activities.map(function (t) {
          return '<li class="chip mono">' + ctx.escapeHtml(t) + '</li>';
        }).join('') + '</ul>'
      : '<p class="detail-muted">None.</p>';

    var bottom = '<h4 class="detail-subhead">Recent claimed runs (' + claimed.length + ')</h4>';
    if (!claimed.length) {
      bottom += '<p class="detail-muted">No runs claimed by this worker in the last 20 results.</p>';
    } else {
      bottom += '<table class="data-table"><thead><tr><th>ID</th><th>Type</th><th>Status</th></tr></thead><tbody>';
      for (var j = 0; j < claimed.length; j++) {
        var run = claimed[j];
        var st = (run.status || '').toUpperCase();
        var runId = ctx.escapeHtml(run.id);
        bottom += '<tr><td class="mono">' +
            '<a href="#" class="cross-link" data-nav="workflow" data-id="' + runId + '">' + runId + '</a>' +
          '</td>' +
          '<td>' + ctx.escapeHtml(run.workflow_type || '-') + '</td>' +
          '<td><span class="badge ' + ctx.badgeClass(st) + '">' + st + '</span></td></tr>';
      }
      bottom += '</tbody></table>';
    }

    return metaStrip +
      '<div class="row-detail-grid row-detail-grid-50-50">' +
        '<div class="row-detail-left">' + leftCol + '</div>' +
        '<div class="row-detail-right">' + rightCol + '</div>' +
      '</div>' +
      '<div class="row-detail-bottom">' + bottom + '</div>';
  }

  function safeParseList(v) {
    if (!v) return [];
    if (Array.isArray(v)) return v;
    try {
      var parsed = JSON.parse(v);
      return Array.isArray(parsed) ? parsed : [];
    } catch (_) { return []; }
  }

  function cssEscape(s) {
    return String(s).replace(/(["\\])/g, '\\$1');
  }

  // Expose setPendingExpand so cross-navigation (ctx.navigate('worker', id))
  // can tell the Workers view which row to auto-expand after its next
  // render. The row is found + clicked inside loadWorkers once data
  // arrives.
  function setPendingExpand(id) {
    expandedWorkerId = id || null;
  }
  if (typeof window !== 'undefined') {
    window.AssayWorkers = { render: render, setPendingExpand: setPendingExpand };
  }

  return { render: render, setPendingExpand: setPendingExpand };
})();
