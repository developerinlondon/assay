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
  // Module-level record of which workflow id (if any) is currently
  // inline-expanded. Preserved across list re-renders triggered by
  // SSE events (workflow_cancelled, workflow_completed, etc.) so the
  // row stays open while the table rebuilds around it. Cleared when
  // the user closes the detail or switches to a different row.
  var expandedWorkflowId = null;

  function toggleRowDetail(linkEl) {
    var row = linkEl.closest('tr');
    if (!row) return;
    var id = linkEl.dataset.id;

    // Click-to-close on the already-expanded row.
    var next = row.nextElementSibling;
    if (next && next.classList.contains('wf-detail-row') && next.dataset.forId === id) {
      next.remove();
      row.classList.remove('wf-row-expanded');
      expandedWorkflowId = null;
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
    expandedWorkflowId = id;

    var target = detailRow.querySelector('.wf-inline-detail');
    if (ctx.showDetail) {
      ctx.showDetail(id, ctx, {
        target: target,
        onClose: function () {
          detailRow.remove();
          row.classList.remove('wf-row-expanded');
          if (expandedWorkflowId === id) expandedWorkflowId = null;
        },
      });
    }
  }

  // Re-open the previously-expanded row after a table re-render. Keeps
  // the operator's active view alive through SSE-triggered refreshes —
  // without this, every workflow_started / workflow_cancelled /
  // workflow_completed event would fold the inline detail the operator
  // was reading.
  function restoreExpandedRow() {
    if (!expandedWorkflowId) return;
    var wrap = container && container.querySelector('#wf-table-wrap');
    if (!wrap) return;
    var link = wrap.querySelector('.wf-link[data-id="' + cssEscape(expandedWorkflowId) + '"]');
    if (!link) {
      // Row no longer on this page (e.g. scrolled off pagination).
      expandedWorkflowId = null;
      return;
    }
    // toggleRowDetail both opens if closed and closes if open — since
    // the fresh render has no expansion, it reopens.
    toggleRowDetail(link);
  }

  function cssEscape(s) {
    return String(s).replace(/(["\\])/g, '\\$1');
  }

  function render(el, context) {
    ctx = context;
    container = el;
    currentOffset = 0;
    currentFilter = '';
    searchTerm = '';
    searchAttrs = '';

    el.innerHTML =
      '<div class="toolbar">' +
        '<h2 class="section-title">Workflows</h2>' +
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
        '<button type="button" class="btn-action btn-advanced-toggle" id="wf-advanced-toggle" ' +
                'title="Show / hide advanced filters">' +
          '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="4" y1="21" x2="4" y2="14"/><line x1="4" y1="10" x2="4" y2="3"/><line x1="12" y1="21" x2="12" y2="12"/><line x1="12" y1="8" x2="12" y2="3"/><line x1="20" y1="21" x2="20" y2="16"/><line x1="20" y1="12" x2="20" y2="3"/><line x1="1" y1="14" x2="7" y2="14"/><line x1="9" y1="8" x2="15" y2="8"/><line x1="17" y1="16" x2="23" y2="16"/></svg>' +
        '</button>' +
        '<button type="button" class="btn-action btn-action-primary" id="wf-start-toggle">+ Start workflow</button>' +
      '</div>' +
      '<div class="toolbar-advanced" id="wf-advanced-row" hidden>' +
        '<input type="text" class="search-input" id="wf-search-attrs" placeholder=\'Filter by search attributes, e.g. {"env":"prod","tenant":"acme"}\' style="flex:1;">' +
        '<span class="inline-form-hint">Matches workflows whose search_attributes contain every listed key at the given value.</span>' +
      '</div>' +
      '<div id="wf-start-form-wrap"></div>' +
      '<div id="wf-table-wrap"></div>' +
      '<div id="wf-pagination" class="pagination"></div>';

    el.querySelector('#wf-search').addEventListener('input', function (e) {
      searchTerm = e.target.value.trim();
      currentOffset = 0;
      loadWorkflows();
    });

    if (typeof AssaySelect !== 'undefined') {
      AssaySelect.enhance(el.querySelector('#wf-status-filter'));
    }

    // Advanced-filters toggle — hides/shows the search_attrs row. The
    // row is hidden by default because most operators don't use search
    // attribute filters; it's a power-user feature for multi-tenant
    // installs where workflows carry tenant / env / project tags.
    el.querySelector('#wf-advanced-toggle').addEventListener('click', function () {
      var row = el.querySelector('#wf-advanced-row');
      var btn = el.querySelector('#wf-advanced-toggle');
      var hidden = row.hasAttribute('hidden');
      if (hidden) {
        row.removeAttribute('hidden');
        btn.classList.add('active');
      } else {
        row.setAttribute('hidden', '');
        btn.classList.remove('active');
        // Clearing the search when collapsing keeps "what the user
        // sees filtered by" consistent with "what's on-screen".
        searchAttrs = '';
        el.querySelector('#wf-search-attrs').value = '';
        loadWorkflows();
      }
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
      // Row action icons — route through AssayActions which owns the
      // modal-driven flows (Signal, Cancel, Terminate, Continue-as-
      // new). Has to come BEFORE the generic-button bail-out below or
      // the button-element check would short-circuit and the action
      // never fires.
      var actionBtn = e.target.closest('.row-action-btn');
      if (actionBtn && ctx.actions) {
        e.preventDefault();
        e.stopPropagation();
        var id = actionBtn.dataset.id;
        var act = actionBtn.dataset.action;
        if (act === 'signal')        ctx.actions.signal(id);
        else if (act === 'cancel')   ctx.actions.cancel(id);
        else if (act === 'terminate') ctx.actions.terminate(id);
        else if (act === 'continue') ctx.actions.continueAsNew(id);
        return;
      }

      // Anything else inside a button (e.g. + Start workflow, pagination)
      // shouldn't trigger the row expansion. Bail out.
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
    // `searchTerm` matches id OR workflow_type, case-insensitive
    // substring. The server's `?type=` param only matches workflow_type
    // EXACTLY, which is why typing "demo" wouldn't match a
    // "DemoPipeline" type or a "demo-1" id — hence filtering client-
    // side against the current page's results. Large-list users still
    // paginate normally; if search coverage across ALL runs is needed,
    // that's a separate backend feature (substring LIKE in the store).
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
      if (searchTerm) {
        var needle = searchTerm.toLowerCase();
        workflows = workflows.filter(function (w) {
          var id = String(w.id || '').toLowerCase();
          var ty = String(w.workflow_type || '').toLowerCase();
          return id.indexOf(needle) !== -1 || ty.indexOf(needle) !== -1;
        });
      }
      renderTable(wrap, workflows);
      renderPagination(workflows.length);
      restoreExpandedRow();
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
        // Full workflow id in the list view — the id is usually the
        // primary thing an operator wants to read. `word-break: break-all`
        // in `.data-table .mono` lets long ids wrap at column boundaries
        // instead of forcing a horizontal scroll. The inline expanded
        // detail hides its own id header because this row already shows
        // the full value.
        '<td><a href="#" class="clickable wf-link mono" data-id="' + ctx.escapeHtml(wf.id) + '">' +
          ctx.escapeHtml(wf.id) + '</a></td>' +
        '<td>' + ctx.escapeHtml(wf.workflow_type || '-') + '</td>' +
        '<td><span class="badge ' + ctx.badgeClass(status) + '">' + status + '</span></td>' +
        '<td class="mono">' + ctx.escapeHtml(wf.task_queue || 'main') + '</td>' +
        '<td>' + ctx.formatTime(wf.created_at) + '</td>' +
        '<td>';

      // Action icons — Signal / Cancel / Terminate / Continue-as-new.
      // Inline SVGs keep the dashboard zero-dependency (no icon font
      // download). title= drives the native tooltip; AssayActions opens
      // the modal flow on click. Terminal runs only get
      // Continue-as-new (the others are no-ops on a finished run).
      var idAttr = ctx.escapeHtml(wf.id);
      html += '<div class="row-actions">';
      if (!terminal) {
        // Radio-broadcast antenna with concentric waves — reads as
        // "send a signal out into the world", not "filter" (which
        // the funnel icon implied).
        html +=
          '<button class="row-action-btn" data-action="signal" data-id="' + idAttr + '" title="Send signal" aria-label="Send signal">' +
            '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">' +
              '<path d="M5 12.55a11 11 0 0 1 14 0"/>' +
              '<path d="M8.5 16.05a6 6 0 0 1 7 0"/>' +
              '<line x1="12" y1="20" x2="12.01" y2="20"/>' +
              '<path d="M1.42 9a16 16 0 0 1 21.16 0"/>' +
            '</svg>' +
          '</button>' +
          '<button class="row-action-btn" data-action="cancel" data-id="' + idAttr + '" title="Cancel — graceful, runs workflow cleanup (SIGTERM-style)" aria-label="Cancel workflow">' +
            '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><line x1="4.93" y1="4.93" x2="19.07" y2="19.07"/></svg>' +
          '</button>' +
          '<button class="row-action-btn row-action-danger" data-action="terminate" data-id="' + idAttr + '" title="Kill — force stop, no cleanup (SIGKILL-style)" aria-label="Kill workflow">' +
            '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>' +
          '</button>';
      }
      // "Start a new run" (continue-as-new) is only offered on
      // terminal workflows. Running it against an in-flight workflow
      // would flip the current run to COMPLETED, truncating real
      // in-progress work — misleading for audit + wrong semantically.
      // Operators who want to restart a live workflow should Cancel
      // or Terminate first (explicit, visible) and then start fresh
      // from the terminal run. The icon is plus-in-play-circle —
      // reads as "new", not "retry".
      if (terminal) {
        html +=
          '<button class="row-action-btn" data-action="continue" data-id="' + idAttr + '" title="Start a new run (continue-as-new)" aria-label="Start a new run">' +
            '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">' +
              '<circle cx="12" cy="12" r="9"/>' +
              '<path d="M10 9v6l5-3-5-3z" fill="currentColor" stroke="none"/>' +
            '</svg>' +
          '</button>';
      }
      html += '</div>';

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
        '<label>Namespace' +
          // Native combobox: defaults to the currently-selected
          // namespace from the sidebar, can be overridden by picking
          // another known namespace OR typing a fresh one (the engine
          // auto-creates namespaces on first use).
          '<input type="text" name="namespace" list="wf-known-namespaces" value="' +
            ctx.escapeHtml(ctx.getNamespace()) + '">' +
          '<datalist id="wf-known-namespaces"></datalist>' +
          '<span class="inline-form-hint">Prefilled with your current namespace. Pick another known one or type a new name.</span>' +
        '</label>' +
        '<label>Task queue' +
          // <input list=...> gives native combobox: pick from known
          // queues OR type a new name. Starts empty (not pre-filled
          // with "default") because browsers filter the datalist by
          // what's typed — pre-filling "default" hides options that
          // don't start with that string. Empty input shows all.
          '<input type="text" name="task_queue" list="wf-known-queues" ' +
                 'placeholder="default (or pick/type a queue)">' +
          '<datalist id="wf-known-queues"></datalist>' +
          '<span class="inline-form-hint">Pick an existing queue or type a new name. Leave blank for "default".</span>' +
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

    // Populate the known-queues datalist so users see what already
    // exists (with worker counts) while still being able to type a
    // new queue name. Fire-and-forget — the input is usable even if
    // the fetch fails.
    ctx.apiFetch('/queues').then(function (queues) {
      var dl = wrap.querySelector('#wf-known-queues');
      if (!dl || !Array.isArray(queues)) return;
      var html = '';
      for (var i = 0; i < queues.length; i++) {
        var q = queues[i];
        var name = ctx.escapeHtml(q.queue || '');
        var workers = q.workers != null ? q.workers : 0;
        html += '<option value="' + name + '" label="' + workers + ' worker' + (workers === 1 ? '' : 's') + '"></option>';
      }
      dl.innerHTML = html;
    }).catch(function () { /* leave empty */ });

    // Populate known-namespaces too so the namespace combobox shows
    // all namespaces the operator can target, not just the currently-
    // selected one. Uses apiFetchRaw because namespaces aren't
    // namespace-scoped.
    ctx.apiFetchRaw('/namespaces').then(function (namespaces) {
      var dl = wrap.querySelector('#wf-known-namespaces');
      if (!dl || !Array.isArray(namespaces)) return;
      var html = '';
      for (var i = 0; i < namespaces.length; i++) {
        var n = namespaces[i];
        var name = ctx.escapeHtml(n.name || n);
        html += '<option value="' + name + '"></option>';
      }
      dl.innerHTML = html;
    }).catch(function () { /* leave empty */ });
  }

  async function handleStart(e) {
    e.preventDefault();
    var form = e.currentTarget;
    var data = new FormData(form);
    // Empty task_queue input → submit "default" (same as before; just
    // moved the fallback down to handle the empty-by-default input).
    var queueRaw = (data.get('task_queue') || '').trim();
    var nsRaw = (data.get('namespace') || '').trim();
    var body = {
      workflow_type: data.get('type').trim(),
      task_queue: queueRaw || 'default',
      namespace: nsRaw || ctx.getNamespace(),
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

  // Action handlers (Signal / Cancel / Terminate / Continue-as-new)
  // moved to components/actions.js as part of the v0.12.0 modal
  // refactor — see ctx.actions.* invoked from the row-action click
  // handler above.

  // Expose setExpandedId so AssayActions.continueAsNew can transfer the
  // inline expansion from the source workflow to the newly-started
  // continuation run after the list refresh lands.
  function setExpandedId(id) {
    expandedWorkflowId = id || null;
  }

  // Pre-populate the search box from outside (used by cross-link
  // navigation: clicking a workflow_type chip on a worker's detail
  // → ctx.navigate('workflow_type', 'DemoPipeline') → here →
  // search shows the filter + the user can clear it visibly).
  function setSearchTerm(term) {
    searchTerm = term || '';
    if (container) {
      var input = container.querySelector('#wf-search');
      if (input) input.value = searchTerm;
      currentOffset = 0;
      loadWorkflows();
    }
  }

  // Expose on window so AssayActions (which loads before workflows.js)
  // can find it.
  if (typeof window !== 'undefined') {
    window.AssayWorkflows = {
      render: render,
      setExpandedId: setExpandedId,
      setSearchTerm: setSearchTerm,
    };
  }

  return {
    render: render,
    setExpandedId: setExpandedId,
    setSearchTerm: setSearchTerm,
  };
})();
