/* Assay Workflow Dashboard - Schedules Component */

var AssaySchedules = (function () {
  'use strict';

  let ctx = null;
  let container = null;
  let showForm = false;
  let editingName = null; // non-null when the form is in patch mode

  function render(el, context) {
    ctx = context;
    container = el;
    showForm = false;
    editingName = null;

    el.innerHTML =
      '<div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 16px;">' +
        '<h2 class="section-title" style="margin-bottom: 0;">Schedules</h2>' +
        '<button class="btn btn-primary btn-sm" id="sched-toggle-form">+ Create</button>' +
      '</div>' +
      '<div id="sched-form-wrap"></div>' +
      '<div id="sched-table-wrap"></div>';

    el.querySelector('#sched-toggle-form').addEventListener('click', function () {
      showForm = !showForm;
      editingName = null;
      renderForm();
    });

    el.querySelector('#sched-table-wrap').addEventListener('click', function (e) {
      var delBtn = e.target.closest('.btn-delete-schedule');
      if (delBtn) {
        e.preventDefault();
        handleDelete(delBtn.dataset.name);
        return;
      }
      var pauseBtn = e.target.closest('.btn-pause-schedule');
      if (pauseBtn) {
        e.preventDefault();
        handleTogglePaused(pauseBtn.dataset.name, pauseBtn.dataset.paused === 'true');
        return;
      }
      var editBtn = e.target.closest('.btn-edit-schedule');
      if (editBtn) {
        e.preventDefault();
        openEditForm(editBtn.dataset.name);
      }
    });

    loadSchedules();
  }

  /// Render the create-or-edit form in-place. When `editingName` is set,
  /// the form is pre-filled with the current schedule's values and the
  /// submit action PATCHes rather than POSTs.
  function renderForm(prefill) {
    var wrap = container.querySelector('#sched-form-wrap');
    if (!showForm) {
      wrap.innerHTML = '';
      return;
    }
    prefill = prefill || {};

    var isEdit = !!editingName;
    var heading = isEdit ? 'Edit schedule: ' + editingName : 'Create schedule';
    var nameField = isEdit
      ? '<input type="text" class="form-input mono" value="' + ctx.escapeHtml(editingName) + '" disabled>'
      : '<input type="text" class="form-input" id="sched-name" placeholder="daily-cleanup" value="' + ctx.escapeHtml(prefill.name || '') + '">';
    var typeField = isEdit
      ? '<input type="text" class="form-input" value="' + ctx.escapeHtml(prefill.workflow_type || '') + '" disabled>'
      : '<input type="text" class="form-input" id="sched-wf-type" placeholder="CleanupJob" value="' + ctx.escapeHtml(prefill.workflow_type || '') + '">';

    wrap.innerHTML =
      '<div class="form-card">' +
        '<h3 style="margin: 0 0 12px;">' + heading + '</h3>' +
        '<div class="form-group">' +
          '<label class="form-label">Name</label>' +
          nameField +
        '</div>' +
        '<div class="form-group">' +
          '<label class="form-label">Workflow Type</label>' +
          typeField +
        '</div>' +
        '<div class="form-group">' +
          '<label class="form-label">Cron Expression</label>' +
          '<input type="text" class="form-input" id="sched-cron" placeholder="0 0 2 * * *" value="' + ctx.escapeHtml(prefill.cron_expr || '') + '">' +
        '</div>' +
        '<div class="form-group">' +
          '<label class="form-label">Timezone (IANA, default UTC)</label>' +
          '<input type="text" class="form-input" id="sched-tz" placeholder="Europe/Berlin" value="' + ctx.escapeHtml(prefill.timezone || '') + '">' +
        '</div>' +
        '<div class="form-group">' +
          '<label class="form-label">Queue</label>' +
          '<input type="text" class="form-input" id="sched-queue" placeholder="main" value="' + ctx.escapeHtml(prefill.task_queue || 'main') + '">' +
        '</div>' +
        '<div class="form-group">' +
          '<label class="form-label">Input (JSON)</label>' +
          '<textarea class="form-textarea" id="sched-input" placeholder="{}">' + ctx.escapeHtml(prefill.input || '') + '</textarea>' +
        '</div>' +
        '<div class="form-group">' +
          '<label class="form-label">Overlap Policy</label>' +
          '<select class="form-select" id="sched-overlap">' +
            overlapOption('skip', prefill.overlap_policy) +
            overlapOption('queue', prefill.overlap_policy) +
            overlapOption('cancel_old', prefill.overlap_policy) +
            overlapOption('allow_all', prefill.overlap_policy) +
          '</select>' +
        '</div>' +
        '<div style="display: flex; gap: 8px;">' +
          '<button class="btn btn-primary" id="sched-submit-btn">' + (isEdit ? 'Save changes' : 'Create schedule') + '</button>' +
          '<button class="btn" id="sched-cancel-btn">Cancel</button>' +
        '</div>' +
      '</div>';

    if (typeof AssaySelect !== 'undefined') {
      AssaySelect.enhance(wrap.querySelector('#sched-overlap'));
    }

    wrap.querySelector('#sched-submit-btn').addEventListener(
      'click',
      isEdit ? handlePatch : handleCreate
    );
    wrap.querySelector('#sched-cancel-btn').addEventListener('click', function () {
      showForm = false;
      editingName = null;
      renderForm();
    });
  }

  function overlapOption(value, current) {
    var selected = current === value ? ' selected' : '';
    return '<option value="' + value + '"' + selected + '>' + value + '</option>';
  }

  async function loadSchedules() {
    var wrap = container.querySelector('#sched-table-wrap');
    try {
      var schedules = await ctx.apiFetch('/schedules');
      renderTable(wrap, schedules || []);
    } catch (err) {
      wrap.innerHTML = '<div class="empty-state"><p>Error: ' + ctx.escapeHtml(err.message) + '</p></div>';
    }
  }

  function renderTable(wrap, schedules) {
    if (schedules.length === 0) {
      wrap.innerHTML = '<div class="empty-state"><p>No schedules configured</p></div>';
      return;
    }

    var html =
      '<table class="data-table"><thead><tr>' +
        '<th>Name</th>' +
        '<th>Workflow Type</th>' +
        '<th>Cron</th>' +
        '<th>Timezone</th>' +
        '<th>Queue</th>' +
        '<th>Paused</th>' +
        '<th>Last Run</th>' +
        '<th>Actions</th>' +
      '</tr></thead><tbody>';

    for (var i = 0; i < schedules.length; i++) {
      var s = schedules[i];
      var paused = !!s.paused;
      html +=
        '<tr>' +
          '<td class="mono">' + ctx.escapeHtml(s.name) + '</td>' +
          '<td>' + ctx.escapeHtml(s.workflow_type) + '</td>' +
          '<td class="mono">' + ctx.escapeHtml(s.cron_expr) + '</td>' +
          '<td>' + ctx.escapeHtml(s.timezone || 'UTC') + '</td>' +
          '<td class="mono">' + ctx.escapeHtml(s.task_queue || 'main') + '</td>' +
          '<td>' + (paused ? '<span class="badge badge-waiting">paused</span>' : '<span class="badge badge-running">active</span>') + '</td>' +
          '<td>' + (s.last_run_at ? ctx.formatTime(s.last_run_at) : '-') + '</td>' +
          '<td>' +
            '<button class="btn btn-sm btn-edit-schedule" data-name="' + ctx.escapeHtml(s.name) + '">Edit</button> ' +
            '<button class="btn btn-sm btn-pause-schedule" data-name="' + ctx.escapeHtml(s.name) + '" data-paused="' + paused + '">' +
              (paused ? 'Resume' : 'Pause') +
            '</button> ' +
            '<button class="btn btn-sm btn-danger btn-delete-schedule" data-name="' + ctx.escapeHtml(s.name) + '">Delete</button>' +
          '</td>' +
        '</tr>';
    }

    html += '</tbody></table>';
    wrap.innerHTML = html;
  }

  async function openEditForm(name) {
    try {
      var s = await ctx.apiFetch('/schedules/' + encodeURIComponent(name));
      if (!s) throw new Error('not found');
      editingName = name;
      showForm = true;
      renderForm({
        name: s.name,
        workflow_type: s.workflow_type,
        cron_expr: s.cron_expr,
        timezone: s.timezone,
        task_queue: s.task_queue,
        input: s.input,
        overlap_policy: s.overlap_policy,
      });
    } catch (err) {
      ctx.toast('Load schedule failed: ' + err.message, 'error');
    }
  }

  async function handleCreate() {
    var name = container.querySelector('#sched-name').value.trim();
    var wfType = container.querySelector('#sched-wf-type').value.trim();
    var cron = container.querySelector('#sched-cron').value.trim();
    var tz = container.querySelector('#sched-tz').value.trim();
    var queue = container.querySelector('#sched-queue').value.trim() || 'main';
    var inputStr = container.querySelector('#sched-input').value.trim();
    var overlap = container.querySelector('#sched-overlap').value;

    if (!name || !wfType || !cron) {
      ctx.toast('Name, workflow type, and cron are required', 'error');
      return;
    }

    var input = null;
    if (inputStr) {
      try { input = JSON.parse(inputStr); } catch (_) {
        ctx.toast('Input is not valid JSON', 'error');
        return;
      }
    }

    var body = {
      name: name,
      namespace: ctx.getNamespace(),
      workflow_type: wfType,
      cron_expr: cron,
      task_queue: queue,
      overlap_policy: overlap,
    };
    if (tz) body.timezone = tz;
    if (input !== null) body.input = input;

    try {
      await ctx.apiFetchRaw('/schedules', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      ctx.toast("Created schedule '" + name + "'", 'success');
      showForm = false;
      editingName = null;
      renderForm();
      loadSchedules();
    } catch (err) {
      ctx.toast('Create failed: ' + err.message, 'error');
    }
  }

  async function handlePatch() {
    var cron = container.querySelector('#sched-cron').value.trim();
    var tz = container.querySelector('#sched-tz').value.trim();
    var queue = container.querySelector('#sched-queue').value.trim();
    var inputStr = container.querySelector('#sched-input').value.trim();
    var overlap = container.querySelector('#sched-overlap').value;

    var patch = {};
    if (cron) patch.cron_expr = cron;
    if (tz) patch.timezone = tz;
    if (queue) patch.task_queue = queue;
    if (overlap) patch.overlap_policy = overlap;
    if (inputStr) {
      try { patch.input = JSON.parse(inputStr); } catch (_) {
        ctx.toast('Input is not valid JSON', 'error');
        return;
      }
    }
    if (Object.keys(patch).length === 0) {
      ctx.toast('No changes to apply', 'info');
      return;
    }

    try {
      await ctx.apiFetch('/schedules/' + encodeURIComponent(editingName), {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(patch),
      });
      ctx.toast('Schedule updated', 'success');
      showForm = false;
      editingName = null;
      renderForm();
      loadSchedules();
    } catch (err) {
      ctx.toast('Update failed: ' + err.message, 'error');
    }
  }

  async function handleTogglePaused(name, currentlyPaused) {
    var action = currentlyPaused ? 'resume' : 'pause';
    try {
      await ctx.apiFetch('/schedules/' + encodeURIComponent(name) + '/' + action, {
        method: 'POST',
      });
      ctx.toast('Schedule ' + (currentlyPaused ? 'resumed' : 'paused'), 'success');
      loadSchedules();
    } catch (err) {
      ctx.toast(action + ' failed: ' + err.message, 'error');
    }
  }

  async function handleDelete(name) {
    if (!confirm('Delete schedule "' + name + '"?')) return;
    try {
      await ctx.apiFetch('/schedules/' + encodeURIComponent(name), { method: 'DELETE' });
      ctx.toast('Schedule deleted', 'success');
      loadSchedules();
    } catch (err) {
      ctx.toast('Delete failed: ' + err.message, 'error');
    }
  }

  return { render: render };
})();
