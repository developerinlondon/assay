/* Assay Workflow Dashboard - Schedules Component */

var AssaySchedules = (function () {
  'use strict';

  let ctx = null;
  let container = null;
  let showForm = false;

  function render(el, context) {
    ctx = context;
    container = el;
    showForm = false;

    el.innerHTML =
      '<div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 16px;">' +
        '<h2 class="section-title" style="margin-bottom: 0;">Schedules</h2>' +
        '<button class="btn btn-primary btn-sm" id="sched-toggle-form">+ Create</button>' +
      '</div>' +
      '<div id="sched-form-wrap"></div>' +
      '<div id="sched-table-wrap"></div>';

    el.querySelector('#sched-toggle-form').addEventListener('click', function () {
      showForm = !showForm;
      renderForm();
    });

    el.querySelector('#sched-table-wrap').addEventListener('click', function (e) {
      var delBtn = e.target.closest('.btn-delete-schedule');
      if (delBtn) {
        e.preventDefault();
        handleDelete(delBtn.dataset.name);
      }
    });

    loadSchedules();
  }

  function renderForm() {
    var wrap = container.querySelector('#sched-form-wrap');
    if (!showForm) {
      wrap.innerHTML = '';
      return;
    }

    wrap.innerHTML =
      '<div class="form-card">' +
        '<div class="form-group">' +
          '<label class="form-label">Name</label>' +
          '<input type="text" class="form-input" id="sched-name" placeholder="daily-cleanup">' +
        '</div>' +
        '<div class="form-group">' +
          '<label class="form-label">Workflow Type</label>' +
          '<input type="text" class="form-input" id="sched-wf-type" placeholder="CleanupJob">' +
        '</div>' +
        '<div class="form-group">' +
          '<label class="form-label">Cron Expression</label>' +
          '<input type="text" class="form-input" id="sched-cron" placeholder="0 * * * *">' +
        '</div>' +
        '<div class="form-group">' +
          '<label class="form-label">Queue</label>' +
          '<input type="text" class="form-input" id="sched-queue" placeholder="main" value="main">' +
        '</div>' +
        '<div class="form-group">' +
          '<label class="form-label">Input (JSON)</label>' +
          '<textarea class="form-textarea" id="sched-input" placeholder="{}"></textarea>' +
        '</div>' +
        '<div class="form-group">' +
          '<label class="form-label">Overlap Policy</label>' +
          '<select class="form-select" id="sched-overlap">' +
            '<option value="skip">Skip</option>' +
            '<option value="queue">Queue</option>' +
            '<option value="cancel_old">Cancel Old</option>' +
            '<option value="allow_all">Allow All</option>' +
          '</select>' +
        '</div>' +
        '<div style="display: flex; gap: 8px;">' +
          '<button class="btn btn-primary" id="sched-create-btn">Create Schedule</button>' +
          '<button class="btn" id="sched-cancel-btn">Cancel</button>' +
        '</div>' +
      '</div>';

    wrap.querySelector('#sched-create-btn').addEventListener('click', handleCreate);
    wrap.querySelector('#sched-cancel-btn').addEventListener('click', function () {
      showForm = false;
      renderForm();
    });
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
        '<th>Queue</th>' +
        '<th>Last Run</th>' +
        '<th>Actions</th>' +
      '</tr></thead><tbody>';

    for (var i = 0; i < schedules.length; i++) {
      var s = schedules[i];
      html +=
        '<tr>' +
          '<td class="mono">' + ctx.escapeHtml(s.name) + '</td>' +
          '<td>' + ctx.escapeHtml(s.workflow_type) + '</td>' +
          '<td class="mono">' + ctx.escapeHtml(s.cron_expr) + '</td>' +
          '<td class="mono">' + ctx.escapeHtml(s.task_queue || 'main') + '</td>' +
          '<td>' + (s.last_run_at ? ctx.formatTime(s.last_run_at) : '-') + '</td>' +
          '<td><button class="btn btn-sm btn-danger btn-delete-schedule" data-name="' +
            ctx.escapeHtml(s.name) + '">Delete</button></td>' +
        '</tr>';
    }

    html += '</tbody></table>';
    wrap.innerHTML = html;
  }

  async function handleCreate() {
    var name = container.querySelector('#sched-name').value.trim();
    var wfType = container.querySelector('#sched-wf-type').value.trim();
    var cron = container.querySelector('#sched-cron').value.trim();
    var queue = container.querySelector('#sched-queue').value.trim() || 'main';
    var inputStr = container.querySelector('#sched-input').value.trim();
    var overlap = container.querySelector('#sched-overlap').value;

    if (!name || !wfType || !cron) {
      alert('Name, Workflow Type, and Cron Expression are required.');
      return;
    }

    var input = null;
    if (inputStr) {
      try { input = JSON.parse(inputStr); } catch (_) {
        alert('Invalid JSON in input field.');
        return;
      }
    }

    var body = {
      name: name,
      namespace: ctx.namespace,
      workflow_type: wfType,
      cron_expr: cron,
      task_queue: queue,
      overlap_policy: overlap,
    };
    if (input !== null) body.input = input;

    try {
      await fetch('/api/v1/schedules', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      showForm = false;
      renderForm();
      loadSchedules();
    } catch (err) {
      alert('Create failed: ' + err.message);
    }
  }

  async function handleDelete(name) {
    if (!confirm('Delete schedule "' + name + '"?')) return;
    try {
      await ctx.apiFetch('/schedules/' + encodeURIComponent(name), { method: 'DELETE' });
      loadSchedules();
    } catch (err) {
      alert('Delete failed: ' + err.message);
    }
  }

  return { render: render };
})();
