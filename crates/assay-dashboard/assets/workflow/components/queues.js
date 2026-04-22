/* Assay Workflow Dashboard — Queues view (v0.12.0)
 *
 * List view + click-through detail. Row click expands inline (same
 * pattern as workflows + workers) showing the workers subscribed to
 * the queue and the recent workflows routed to it. Gives operators a
 * diagnostic view for "why isn't my queue progressing?" without
 * diving into logs.
 */

var AssayQueues = (function () {
  'use strict';

  let ctx = null;
  let container = null;
  let expandedQueueName = null;

  function render(el, context) {
    ctx = context;
    container = el;

    el.innerHTML =
      '<div class="toolbar">' +
        '<h2 class="section-title">Queues</h2>' +
      '</div>' +
      '<div id="queues-table-wrap"></div>';

    loadQueues();

    el.addEventListener('click', function (e) {
      var cross = e.target.closest('.cross-link[data-nav]');
      if (cross) {
        e.preventDefault();
        e.stopPropagation();
        if (ctx.navigate) ctx.navigate(cross.dataset.nav, cross.dataset.id);
        return;
      }
      var row = e.target.closest('tr[data-queue-name]');
      if (!row) return;
      e.preventDefault();
      toggleRowDetail(row);
    });
  }

  async function loadQueues() {
    var wrap = container.querySelector('#queues-table-wrap');
    try {
      var queues = await ctx.apiFetch('/queues');
      renderTable(wrap, queues || []);
      if (expandedQueueName) {
        var row = wrap.querySelector('tr[data-queue-name="' + cssEscape(expandedQueueName) + '"]');
        if (row) toggleRowDetail(row);
        else expandedQueueName = null;
      }
    } catch (err) {
      wrap.innerHTML = '<div class="empty-state"><p>Error: ' + ctx.escapeHtml(err.message) + '</p></div>';
    }
  }

  function renderTable(wrap, queues) {
    if (queues.length === 0) {
      wrap.innerHTML = '<div class="empty-state"><p>No queues found</p></div>';
      return;
    }

    var html =
      '<table class="data-table"><thead><tr>' +
        '<th>Queue</th>' +
        '<th>Pending</th>' +
        '<th>Running</th>' +
        '<th>Workers</th>' +
      '</tr></thead><tbody>';

    for (var i = 0; i < queues.length; i++) {
      var q = queues[i];
      var name = q.queue || q.name || '-';
      var warn = (q.pending_activities > 0 && q.workers === 0);

      html +=
        '<tr class="clickable-row" data-queue-name="' + ctx.escapeHtml(name) + '">' +
          '<td class="mono clickable">' + ctx.escapeHtml(name) + '</td>' +
          '<td>' + (q.pending_activities || 0) +
            (warn ? ' <span class="warning-icon" title="Backlog with no workers available">&#9888;</span>' : '') +
          '</td>' +
          '<td>' + (q.running_activities || 0) + '</td>' +
          '<td>' + (q.workers || 0) + '</td>' +
        '</tr>';
    }

    html += '</tbody></table>';
    wrap.innerHTML = html;
  }

  // ── Inline row expansion ────────────────────────────────────────
  function toggleRowDetail(row) {
    var name = row.dataset.queueName;
    var next = row.nextElementSibling;
    if (next && next.classList.contains('queue-detail-row') &&
        next.dataset.forName === name) {
      next.remove();
      row.classList.remove('wf-row-expanded');
      expandedQueueName = null;
      return;
    }
    var openDetails = row.parentNode.querySelectorAll('.queue-detail-row');
    for (var i = 0; i < openDetails.length; i++) openDetails[i].remove();
    var openParents = row.parentNode.querySelectorAll('.wf-row-expanded');
    for (var j = 0; j < openParents.length; j++) {
      openParents[j].classList.remove('wf-row-expanded');
    }
    var colCount = row.children.length;
    var detailRow = document.createElement('tr');
    detailRow.className = 'queue-detail-row wf-detail-row';
    detailRow.dataset.forName = name;
    detailRow.innerHTML = '<td colspan="' + colCount + '">' +
      '<div class="wf-inline-detail"><p class="detail-muted">Loading queue detail…</p></div>' +
    '</td>';
    row.parentNode.insertBefore(detailRow, row.nextSibling);
    row.classList.add('wf-row-expanded');
    expandedQueueName = name;
    populateDetail(detailRow.querySelector('.wf-inline-detail'), name);
  }

  async function populateDetail(target, name) {
    try {
      var [queues, workers, workflows] = await Promise.all([
        ctx.apiFetch('/queues'),
        ctx.apiFetch('/workers').catch(function () { return []; }),
        ctx.apiFetch('/workflows?limit=50').catch(function () { return []; }),
      ]);
      var q = Array.isArray(queues) ? queues.find(function (x) { return (x.queue || x.name) === name; }) : null;
      if (!q) throw new Error('queue ' + name + ' not found');
      var attached = (workers || []).filter(function (w) { return w.task_queue === name; });
      var recent = (workflows || []).filter(function (w) { return w.task_queue === name; }).slice(0, 20);
      target.innerHTML = renderDetailHtml(q, attached, recent);
    } catch (err) {
      target.innerHTML = '<div class="error-box">' + ctx.escapeHtml(err.message) + '</div>';
    }
  }

  function renderDetailHtml(q, workers, recent) {
    var now = Date.now() / 1000;

    // Queue row already shows Pending / Running / Workers counts —
    // no need to duplicate them in a left column. The expansion
    // surfaces the things the row CAN'T carry: which workers are
    // subscribed, and what's recently been routed here. Backlog
    // warning floats at the top in red when applicable.
    var top = '';
    if (q.pending_activities > 0 && workers.length === 0) {
      top += '<div class="error-box" style="background: color-mix(in srgb, var(--orange) 12%, transparent); color: var(--orange); border-color: var(--orange); margin-bottom: 12px;">' +
        '&#9888; <strong>Backlog with no workers.</strong> ' +
        'Pending tasks on this queue will stall until a worker registers.' +
        '</div>';
    }

    var workersSection = '<h4 class="detail-subhead">Workers subscribed (' + workers.length + ')</h4>';
    if (!workers.length) {
      workersSection += '<p class="detail-muted">No worker is currently subscribed to this queue.</p>';
    } else {
      workersSection += '<table class="data-table"><thead><tr>' +
        '<th>Status</th><th>ID</th><th>Identity</th><th>Active</th><th>Heartbeat</th></tr></thead><tbody>';
      for (var j = 0; j < workers.length; j++) {
        var w = workers[j];
        var hbAge = now - (w.last_heartbeat || 0);
        var dot = hbAge < 30 ? 'healthy' : hbAge < 60 ? 'warning' : 'stale';
        var cap = w.max_concurrent_workflows || w.max_concurrent_activities || '-';
        var wid = ctx.escapeHtml(w.id);
        workersSection += '<tr>' +
          '<td><span class="worker-dot ' + dot + '"></span></td>' +
          '<td class="mono">' +
            '<a href="#" class="cross-link" data-nav="worker" data-id="' + wid + '" title="' + wid + '">' +
              ctx.escapeHtml(ctx.truncate(w.id, 22)) +
            '</a>' +
          '</td>' +
          '<td>' + ctx.escapeHtml(w.identity || '-') + '</td>' +
          '<td>' + (w.active_tasks || 0) + '/' + cap + '</td>' +
          '<td>' + ctx.formatTime(w.last_heartbeat) + '</td>' +
        '</tr>';
      }
      workersSection += '</tbody></table>';
    }

    var recentSection = '<h4 class="detail-subhead">Recent workflows on this queue (' + recent.length + ')</h4>';
    if (!recent.length) {
      recentSection += '<p class="detail-muted">No workflows routed here in the last 50 results.</p>';
    } else {
      recentSection += '<table class="data-table"><thead><tr><th>ID</th><th>Type</th><th>Status</th><th>Created</th></tr></thead><tbody>';
      for (var k = 0; k < recent.length; k++) {
        var run = recent[k];
        var st = (run.status || '').toUpperCase();
        var rid = ctx.escapeHtml(run.id);
        recentSection += '<tr>' +
          '<td class="mono">' +
            '<a href="#" class="cross-link" data-nav="workflow" data-id="' + rid + '" title="' + rid + '">' +
              ctx.escapeHtml(ctx.truncate(run.id, 28)) +
            '</a>' +
          '</td>' +
          '<td>' + ctx.escapeHtml(run.workflow_type || '-') + '</td>' +
          '<td><span class="badge ' + ctx.badgeClass(st) + '">' + st + '</span></td>' +
          '<td>' + ctx.formatTime(run.created_at) + '</td>' +
        '</tr>';
      }
      recentSection += '</tbody></table>';
    }

    return top + workersSection + recentSection;
  }

  function cssEscape(s) {
    return String(s).replace(/(["\\])/g, '\\$1');
  }

  function setPendingExpand(name) {
    expandedQueueName = name || null;
  }
  if (typeof window !== 'undefined') {
    window.AssayQueues = { render: render, setPendingExpand: setPendingExpand };
  }

  return { render: render, setPendingExpand: setPendingExpand };
})();
