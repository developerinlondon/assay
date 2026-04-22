/* Assay Workflow Dashboard — Custom select component (v0.12.0)
 *
 * Native <select> dropdowns render with the OS's bare-bones widget.
 * AssaySelect.enhance(el) overlays a styled trigger + popup menu on
 * top while keeping the underlying <select> as the source of truth
 * for value + change events — so any existing code that does
 * `select.value = X` or `addEventListener('change', …)` keeps working
 * unchanged.
 *
 * Re-call enhance() any time the underlying <option> list changes
 * (e.g. after a namespace add/delete). The wrapper detects an
 * existing enhancement on the same element and rebuilds the menu in
 * place rather than stacking duplicates.
 */

var AssaySelect = (function () {
  'use strict';

  function enhance(selectEl, opts) {
    if (!selectEl || selectEl.tagName !== 'SELECT') return;
    opts = opts || {};

    // Tear down any prior enhancement on this element first so a
    // re-enhance after option-list changes doesn't stack wrappers.
    var existing = selectEl.parentElement && selectEl.parentElement.classList.contains('assay-select')
      ? selectEl.parentElement
      : null;
    if (existing) {
      var oldUI = existing.querySelector('.assay-select-trigger, .assay-select-menu');
      while (oldUI) {
        oldUI.remove();
        oldUI = existing.querySelector('.assay-select-trigger, .assay-select-menu');
      }
      buildUI(existing, selectEl, opts);
      return;
    }

    var wrapper = document.createElement('div');
    wrapper.className = 'assay-select';
    if (opts.className) wrapper.className += ' ' + opts.className;
    selectEl.parentNode.insertBefore(wrapper, selectEl);
    wrapper.appendChild(selectEl);
    selectEl.classList.add('assay-select-native');
    buildUI(wrapper, selectEl, opts);
  }

  function buildUI(wrapper, selectEl, opts) {
    var current = selectEl.options[selectEl.selectedIndex];
    var label = current ? current.textContent : '';

    var trigger = document.createElement('button');
    trigger.type = 'button';
    trigger.className = 'assay-select-trigger';
    trigger.setAttribute('aria-haspopup', 'listbox');
    trigger.setAttribute('aria-expanded', 'false');
    if (opts.title) trigger.title = opts.title;
    trigger.innerHTML =
      '<span class="assay-select-value">' + escapeHtml(label) + '</span>' +
      '<svg class="assay-select-caret" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 9l6 6 6-6"/></svg>';
    wrapper.appendChild(trigger);

    var menu = document.createElement('div');
    menu.className = 'assay-select-menu';
    menu.setAttribute('role', 'listbox');
    wrapper.appendChild(menu);

    function renderMenu() {
      var html = '';
      for (var i = 0; i < selectEl.options.length; i++) {
        var o = selectEl.options[i];
        var sel = i === selectEl.selectedIndex ? ' aria-selected="true"' : '';
        html += '<button type="button" class="assay-select-option" data-value="' + escapeHtml(o.value) + '" role="option"' + sel + '>' +
                  escapeHtml(o.textContent) +
                '</button>';
      }
      menu.innerHTML = html;
    }

    function syncTriggerLabel() {
      var cur = selectEl.options[selectEl.selectedIndex];
      var v = trigger.querySelector('.assay-select-value');
      if (v) v.textContent = cur ? cur.textContent : '';
    }

    function open() {
      renderMenu();
      wrapper.classList.add('open');
      trigger.setAttribute('aria-expanded', 'true');
      // Close on outside click / Escape.
      setTimeout(function () {
        document.addEventListener('click', onDocClick);
        document.addEventListener('keydown', onKey);
      }, 0);
    }
    function close() {
      wrapper.classList.remove('open');
      trigger.setAttribute('aria-expanded', 'false');
      document.removeEventListener('click', onDocClick);
      document.removeEventListener('keydown', onKey);
    }
    function onDocClick(e) {
      if (!wrapper.contains(e.target)) close();
    }
    function onKey(e) {
      if (e.key === 'Escape') { close(); trigger.focus(); }
    }

    trigger.addEventListener('click', function (e) {
      e.stopPropagation();
      if (wrapper.classList.contains('open')) close();
      else open();
    });

    menu.addEventListener('click', function (e) {
      var btn = e.target.closest('.assay-select-option');
      if (!btn) return;
      var v = btn.dataset.value;
      if (selectEl.value !== v) {
        selectEl.value = v;
        // Programmatic value-set doesn't fire change; dispatch one so
        // existing change-listeners (namespace switcher, etc.) react.
        selectEl.dispatchEvent(new Event('change', { bubbles: true }));
      }
      syncTriggerLabel();
      close();
    });

    // External code may set selectEl.value programmatically (e.g.
    // namespace mirroring between sidebar + status bar). Sync the
    // trigger label whenever the underlying select changes.
    selectEl.addEventListener('change', syncTriggerLabel);
  }

  // Attribute-safe escape (covers ", ' too — option values may contain
  // them, e.g. namespaces with apostrophes). textContent → innerHTML
  // only handles <, >, & which is insufficient for attribute contexts.
  function escapeHtml(s) {
    return String(s == null ? '' : s)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#39;');
  }

  return { enhance: enhance };
})();
