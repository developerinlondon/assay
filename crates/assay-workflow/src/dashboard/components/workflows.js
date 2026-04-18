/* Assay Workflow Dashboard - Workflows Component */

var AssayWorkflows = (function () {
  'use strict';

  const PAGE_SIZE = 20;
  let currentOffset = 0;
  let currentFilter = '';
  let searchTerm = '';
  let searchAttrs = '';
  let ctx = null;
  let container = null;

  /**
   * Toggle the inline expansion row for a workflow. Click once to open,
   * click again to close. Opening another row auto-closes the previous
   * one — simpler to scan than a pile of open rows, matches the pattern
   * you'd get from a radio group.
   */
  function toggleRowDetail(linkEl) {
    var row = linkEl.closest('tr');
    if (!row) return;
    var id = linkEl.dataset.id;

    // Click-to-close on the already-expanded row.
    var next = row.nextElementSibling;
    if (next && next.classList.contains('wf-detail-row') && next.dataset.forId === id) {
      next.remove();
      row.classList.remove('wf-row-expanded');
      return;
    }
    // Close any other open detail rows in the same table first.
    var openDetails = row.parentNode.querySelectorAll('.wf-detail-row');
    for (var i = 0; i < openDetails.length; i++) openDetails[i].remove();
    var openParents = row.parentNode.querySelectorAll('.wf-row-expanded');
    for (var j = 0; j < openParents.length; j++) {
      openParents[j].classList.remove('wf-row-expanded');
    }

    // Expand this row.
    var colCount = row.children.length;
    var detailRow = document.createElement('tr');
    detailRow.className = 'wf-detail-row';
    detailRow.dataset.forId = id;
    detailRow.innerHTML =
      '<td colspan="' + colCount + '">' +
        '<div class="wf-inline-detail"></div>' +
      '</td>';
    row.parentNode.insertBefore(detailRow, row.nextSibling);
    row.classList.add('wf-row-expanded');

    var target = detailRow.querySelector('.wf-inline-detail');
    if (ctx.showDetail) {
      ctx.showDetail(id, ctx, {
        target: target,
        onClose: function () {
          detailRow.remove();
          row.classList.remove('wf-row-expanded');
        },
      });
    }
  }

  function render(el, context) {
    ctx = context;
    container = el;
    currentOffset = 0;
    currentFilter = '';
    searchTerm = '';
    searchAttrs = '';

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
        '<input type="text" class="search-input" id="wf-search-attrs" placeholder=\'Search attrs filter, e.g. {"env":"prod"}\' style="flex:1.2;">' +
        '<button type="button" class="btn-action btn-action-primary" id="wf-start-toggle">+ Start workflow</button>' +
      '</div>' +
      '<div id="wf-start-form-wrap"></div>' +
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

    // Search-attributes filter: debounce so every keystroke doesn't hit
    // the API. 300ms matches common search-field latency heuristics.
    let searchAttrsTimer = null;
    el.querySelector('#wf-search-attrs').addEventListener('input', function (e) {
      const val = e.target.value.trim();
      clearTimeout(searchAttrsTimer);
      searchAttrsTimer = setTimeout(function () {
        searchAttrs = val;
        currentOffset = 0;
        loadWorkflows();
      }, 300);
    });

    el.querySelector('#wf-start-toggle').addEventListener('click', toggleStartForm);

    el.querySelector('#wf-table-wrap').addEventListener('click', function (e) {
      // Button / interactive elements inside the row — Signal, Cancel,
      // Terminate etc — shouldn't trigger the row expansion. Bail out
      // when the click originates inside anything that already owns a
      // click behaviour.
      if (e.target.closest('button, .btn')) return;

      // The id link still fires first (delegated below). Any other
      // click inside a workflow row (body text, badges, queue name)
      // expands the row too, so users can click anywhere on the row —
      // not just the link — to drill in.
      var link = e.target.closest('.wf-link');
      if (link) {
        e.preventDefault();
        toggleRowDetail(link);
        return;
      }

      // Click anywhere else on a workflow row — fall through to row
      // expansion using the row's first .wf-link as the trigger source.
      var row = e.target.closest('tr');
      if (row && row.classList.contains('clickable-row')) {
        var anyLink = row.querySelector('.wf-link');
        if (anyLink) {
          toggleRowDetail(anyLink);
          return;
        }
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
        return;
      }

      var termBtn = e.target.closest('.btn-terminate');
      if (termBtn) {
        e.preventDefault();
        handleTerminate(termBtn.dataset.id);
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
    if (searchAttrs) {
      // Validate JSON client-side so bad input doesn't silently vanish.
      try {
        JSON.parse(searchAttrs);
        params += '&search_attrs=' + encodeURIComponent(searchAttrs);
      } catch (_) {
        // Invalid JSON: skip the param, leave a subtle hint on the input.
        var attrsInput = container.querySelector('#wf-search-attrs');
        if (attrsInput) attrsInput.style.borderColor = '#d04040';
        renderTable(wrap, []);
        return;
      }
    }
    var attrsInput = container.querySelector('#wf-search-attrs');
    if (attrsInput) attrsInput.style.borderColor = '';

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
        '<tr class="clickable-row">' +
        // title= reveals the full workflow id on hover — ids are
        // truncated to 32 chars for table density, but operators debugging
        // a specific run need to see the whole value without opening the
        // detail panel or URL-bar surgery.
        '<td><a href="#" class="clickable wf-link mono" data-id="' + ctx.escapeHtml(wf.id) +
          '" title="' + ctx.escapeHtml(wf.id) + '">' +
          ctx.escapeHtml(ctx.truncate(wf.id, 32)) + '</a></td>' +
        '<td>' + ctx.escapeHtml(wf.workflow_type || '-') + '</td>' +
        '<td><span class="badge ' + ctx.badgeClass(status) + '">' + status + '</span></td>' +
        '<td class="mono">' + ctx.escapeHtml(wf.task_queue || 'main') + '</td>' +
        '<td>' + ctx.formatTime(wf.created_at) + '</td>' +
        '<td>';

      if (!terminal) {
        html +=
          '<button class="btn btn-sm btn-signal" data-id="' + ctx.escapeHtml(wf.id) + '">Signal</button> ' +
          '<button class="btn btn-sm btn-cancel" data-id="' + ctx.escapeHtml(wf.id) + '">Cancel</button> ' +
          '<button class="btn btn-sm btn-danger btn-terminate" data-id="' + ctx.escapeHtml(wf.id) + '">Terminate</button>';
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

    // Don't render the pagination bar at all when there's only one
    // page of results (no prev page, current page isn't full). A
    // standalone "Page 1" on a three-row table is chrome-for-its-own-
    // sake; operators only need the controls once there's actually
    // something to page through.
    var hasPrev = currentOffset > 0;
    var hasNext = count === PAGE_SIZE;
    if (!hasPrev && !hasNext) {
      pag.innerHTML = '';
      return;
    }

    var html = '';
    if (hasPrev) {
      html += '<button class="btn btn-sm" data-offset="' + Math.max(0, currentOffset - PAGE_SIZE) + '">Prev</button>';
    }
    var page = Math.floor(currentOffset / PAGE_SIZE) + 1;
    html += '<span style="padding: 0 8px; color: var(--text-muted);">Page ' + page + '</span>';
    if (hasNext) {
      html += '<button class="btn btn-sm" data-offset="' + (currentOffset + PAGE_SIZE) + '">Next</button>';
    }
    pag.innerHTML = html;
  }

  /// Open/close the inline "start workflow" form. Collapsed by default so
  /// the list takes the full width; click the button to expand.
  function toggleStartForm() {
    var wrap = container.querySelector('#wf-start-form-wrap');
    if (wrap.innerHTML.trim() !== '') {
      wrap.innerHTML = '';
      return;
    }
    wrap.innerHTML =
      '<form class="inline-form" id="wf-start-form">' +
        '<label>Workflow type <span style="color:#d04040">*</span>' +
          '<input type="text" name="type" placeholder="e.g. IngestData" required>' +
        '</label>' +
        '<label>Workflow ID (optional — auto-generated if blank)' +
          '<input type="text" name="id" placeholder="e.g. ingest-2026-04-17">' +
        '</label>' +
        '<label>Task queue' +
          '<input type="text" name="task_queue" value="default">' +
        '</label>' +
        '<label>Input (JSON, optional)' +
          '<textarea name="input" placeholder=\'{"key":"value"}\'></textarea>' +
        '</label>' +
        '<label>Search attributes (JSON, optional)' +
          '<textarea name="search_attrs" placeholder=\'{"env":"prod","tenant":"acme"}\'></textarea>' +
        '</label>' +
        '<div class="form-actions">' +
          '<button type="button" class="btn-action" id="wf-start-cancel">Cancel</button>' +
          '<button type="submit" class="btn-action btn-action-primary">Start</button>' +
        '</div>' +
      '</form>';
    wrap.querySelector('#wf-start-cancel').addEventListener('click', function () {
      wrap.innerHTML = '';
    });
    wrap.querySelector('#wf-start-form').addEventListener('submit', handleStart);
  }

  async function handleStart(e) {
    e.preventDefault();
    var form = e.currentTarget;
    var data = new FormData(form);
    var body = {
      workflow_type: data.get('type').trim(),
      task_queue: (data.get('task_queue') || 'default').trim(),
      namespace: ctx.getNamespace(),
    };
    var idVal = (data.get('id') || '').trim();
    if (idVal) {
      body.workflow_id = idVal;
    } else {
      body.workflow_id =
        'wf-' + body.workflow_type.toLowerCase() + '-' + Date.now();
    }
    var inputRaw = (data.get('input') || '').trim();
    if (inputRaw) {
      try {
        body.input = JSON.parse(inputRaw);
      } catch (err) {
        ctx.toast('Input is not valid JSON', 'error');
        return;
      }
    }
    var attrsRaw = (data.get('search_attrs') || '').trim();
    if (attrsRaw) {
      try {
        body.search_attributes = JSON.parse(attrsRaw);
      } catch (err) {
        ctx.toast('Search attributes must be valid JSON', 'error');
        return;
      }
    }

    try {
      await ctx.apiFetchRaw('/workflows', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      ctx.toast('Started ' + body.workflow_id, 'success');
      container.querySelector('#wf-start-form-wrap').innerHTML = '';
      loadWorkflows();
    } catch (err) {
      ctx.toast('Start failed: ' + err.message, 'error');
    }
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
      ctx.toast("Signal '" + name + "' sent", 'success');
      loadWorkflows();
    } catch (err) {
      ctx.toast('Signal failed: ' + err.message, 'error');
    }
  }

  async function handleCancel(id) {
    if (!confirm('Cancel workflow ' + id + '?')) return;
    try {
      await ctx.apiFetch('/workflows/' + encodeURIComponent(id) + '/cancel', {
        method: 'POST',
      });
      ctx.toast('Cancel requested', 'success');
      loadWorkflows();
    } catch (err) {
      ctx.toast('Cancel failed: ' + err.message, 'error');
    }
  }

  async function handleTerminate(id) {
    var reason = prompt(
      'Terminate workflow ' + id + '?\n\nReason (optional):',
      ''
    );
    if (reason === null) return; // user cancelled
    var body = reason ? { reason: reason } : {};
    try {
      await ctx.apiFetch('/workflows/' + encodeURIComponent(id) + '/terminate', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      ctx.toast('Terminated', 'success');
      loadWorkflows();
    } catch (err) {
      ctx.toast('Terminate failed: ' + err.message, 'error');
    }
  }

  return { render: render };
})();
