/* Assay Workflow Dashboard - Workflows Component */

var AssayWorkflows = (function () {
  'use strict';

  const PAGE_SIZE = 20;
  let currentOffset = 0;
  let currentFilter = '';
  let searchTerm = '';
  let ctx = null;
  let container = null;

  function render(el, context) {
    ctx = context;
    container = el;
    currentOffset = 0;
    currentFilter = '';
    searchTerm = '';

    el.innerHTML =
      '<h2 class="section-title">Workflows</h2>' +
      '<div class="toolbar">' +
        '<input type="text" class="search-input" id="wf-search" placeholder="Search by ID or type...">' +
        '<select class="filter-select" id="wf-status-filter">' +
          '<option value="">All Statuses</option>' +
          '<option value="PENDING">Pending</option>' +
          '<option value="RUNNING">Running</option>' +
          '<option value="COMPLETED">Completed</option>' +
          '<option value="FAILED">Failed</option>' +
          '<option value="WAITING">Waiting</option>' +
          '<option value="CANCELLED">Cancelled</option>' +
        '</select>' +
      '</div>' +
      '<div id="wf-table-wrap"></div>' +
      '<div id="wf-pagination" class="pagination"></div>';

    el.querySelector('#wf-search').addEventListener('input', function (e) {
      searchTerm = e.target.value.trim();
      currentOffset = 0;
      loadWorkflows();
    });

    el.querySelector('#wf-status-filter').addEventListener('change', function (e) {
      currentFilter = e.target.value;
      currentOffset = 0;
      loadWorkflows();
    });

    el.querySelector('#wf-table-wrap').addEventListener('click', function (e) {
      var link = e.target.closest('.wf-link');
      if (link) {
        e.preventDefault();
        if (ctx.showDetail) ctx.showDetail(link.dataset.id, ctx);
        return;
      }

      var signalBtn = e.target.closest('.btn-signal');
      if (signalBtn) {
        e.preventDefault();
        handleSignal(signalBtn.dataset.id);
        return;
      }

      var cancelBtn = e.target.closest('.btn-cancel');
      if (cancelBtn) {
        e.preventDefault();
        handleCancel(cancelBtn.dataset.id);
      }
    });

    el.querySelector('#wf-pagination').addEventListener('click', function (e) {
      var btn = e.target.closest('.btn[data-offset]');
      if (!btn) return;
      currentOffset = parseInt(btn.dataset.offset, 10);
      loadWorkflows();
    });

    loadWorkflows();
  }

  async function loadWorkflows() {
    var wrap = container.querySelector('#wf-table-wrap');
    var params = '?limit=' + PAGE_SIZE + '&offset=' + currentOffset;
    if (currentFilter) params += '&status=' + currentFilter;
    if (searchTerm) params += '&type=' + encodeURIComponent(searchTerm);

    try {
      var workflows = await ctx.apiFetch('/workflows' + params);
      renderTable(wrap, workflows);
      renderPagination(workflows.length);
    } catch (err) {
      wrap.innerHTML = '<div class="empty-state"><p>Error loading workflows: ' + ctx.escapeHtml(err.message) + '</p></div>';
    }
  }

  function renderTable(wrap, workflows) {
    if (!workflows || workflows.length === 0) {
      wrap.innerHTML = '<div class="empty-state"><p>No workflows found</p></div>';
      return;
    }

    var html =
      '<table class="data-table">' +
      '<thead><tr>' +
        '<th>ID</th>' +
        '<th>Type</th>' +
        '<th>Status</th>' +
        '<th>Queue</th>' +
        '<th>Created</th>' +
        '<th>Actions</th>' +
      '</tr></thead><tbody>';

    for (var i = 0; i < workflows.length; i++) {
      var wf = workflows[i];
      var status = (wf.status || 'PENDING').toUpperCase();
      var terminal = ctx.isTerminal(status);

      html +=
        '<tr>' +
        '<td><a href="#" class="clickable wf-link mono" data-id="' + ctx.escapeHtml(wf.id) + '">' +
          ctx.escapeHtml(ctx.truncate(wf.id, 32)) + '</a></td>' +
        '<td>' + ctx.escapeHtml(wf.workflow_type || '-') + '</td>' +
        '<td><span class="badge ' + ctx.badgeClass(status) + '">' + status + '</span></td>' +
        '<td class="mono">' + ctx.escapeHtml(wf.task_queue || 'main') + '</td>' +
        '<td>' + ctx.formatTime(wf.created_at) + '</td>' +
        '<td>';

      if (!terminal) {
        html +=
          '<button class="btn btn-sm btn-signal" data-id="' + ctx.escapeHtml(wf.id) + '">Signal</button> ' +
          '<button class="btn btn-sm btn-danger btn-cancel" data-id="' + ctx.escapeHtml(wf.id) + '">Cancel</button>';
      } else {
        html += '<span style="color: var(--text-muted)">-</span>';
      }

      html += '</td></tr>';
    }

    html += '</tbody></table>';
    wrap.innerHTML = html;
  }

  function renderPagination(count) {
    var pag = container.querySelector('#wf-pagination');
    var html = '';

    if (currentOffset > 0) {
      html += '<button class="btn btn-sm" data-offset="' + Math.max(0, currentOffset - PAGE_SIZE) + '">Prev</button>';
    }

    var page = Math.floor(currentOffset / PAGE_SIZE) + 1;
    html += '<span style="padding: 0 8px; color: var(--text-muted);">Page ' + page + '</span>';

    if (count === PAGE_SIZE) {
      html += '<button class="btn btn-sm" data-offset="' + (currentOffset + PAGE_SIZE) + '">Next</button>';
    }

    pag.innerHTML = html;
  }

  async function handleSignal(id) {
    var name = prompt('Signal name:');
    if (!name) return;
    var payloadStr = prompt('Signal payload (JSON, or leave empty):', '');
    var payload = null;
    if (payloadStr) {
      try {
        payload = JSON.parse(payloadStr);
      } catch (_) {
        payload = payloadStr;
      }
    }

    try {
      await ctx.apiFetch('/workflows/' + encodeURIComponent(id) + '/signal/' + encodeURIComponent(name), {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ payload: payload }),
      });
      loadWorkflows();
    } catch (err) {
      alert('Signal failed: ' + err.message);
    }
  }

  async function handleCancel(id) {
    if (!confirm('Cancel workflow ' + id + '?')) return;
    try {
      await ctx.apiFetch('/workflows/' + encodeURIComponent(id) + '/cancel', {
        method: 'POST',
      });
      loadWorkflows();
    } catch (err) {
      alert('Cancel failed: ' + err.message);
    }
  }

  return { render: render };
})();
