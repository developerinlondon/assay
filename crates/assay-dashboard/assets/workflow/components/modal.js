/* Assay Workflow Dashboard — Modal component (v0.12.0)
 *
 * Replaces browser prompt() / confirm() with a styled overlay modal.
 * Two flavours:
 *
 *   AssayModal.confirm({ title, message, confirmLabel?, danger?, onConfirm })
 *   AssayModal.form({ title, fields:[…], submitLabel?, danger?, onSubmit(values) })
 *
 * The form variant takes a list of field descriptors:
 *   { name, label, type:'text'|'textarea'|'json', placeholder?, value?, required?, hint? }
 * On submit, JSON fields are parsed and validated; if invalid, the field
 * gets an inline error and the modal stays open. onSubmit receives an
 * object keyed by field name.
 *
 * Dismiss: click the X, click the backdrop, press Escape, or call
 * AssayModal.close() from inside an onConfirm/onSubmit handler.
 */

var AssayModal = (function () {
  'use strict';

  var overlayEl = null;
  var keydownHandler = null;
  var closeCallback = null;

  function ensureOverlay() {
    if (overlayEl) return overlayEl;
    overlayEl = document.createElement('div');
    overlayEl.className = 'modal-overlay';
    overlayEl.setAttribute('aria-hidden', 'true');
    overlayEl.addEventListener('click', function (e) {
      // Click on backdrop (not the dialog) closes.
      if (e.target === overlayEl) close();
    });
    document.body.appendChild(overlayEl);
    return overlayEl;
  }

  // textContent → innerHTML only escapes <, >, &. We render strings into
  // attribute values (placeholder="...", title="..."), where literal "
  // would close the attribute prematurely — e.g. a placeholder of
  // `{ "version": "v0.2.0" }` rendered to `placeholder="{ "version"...`
  // and the browser only sees `{ ` before bailing. Full attribute-safe
  // escape covers all five HTML-special chars.
  function escapeHtml(s) {
    return String(s == null ? '' : s)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#39;');
  }

  function renderFieldHtml(f) {
    var name = escapeHtml(f.name);
    var label = escapeHtml(f.label || f.name);
    var hint = f.hint ? '<div class="modal-field-hint">' + escapeHtml(f.hint) + '</div>' : '';
    var placeholder = f.placeholder ? ' placeholder="' + escapeHtml(f.placeholder) + '"' : '';
    var required = f.required ? ' required' : '';
    var value = f.value != null ? escapeHtml(f.value) : '';
    var control;
    if (f.type === 'textarea' || f.type === 'json') {
      control = '<textarea class="modal-field-control" name="' + name + '" rows="6"' + placeholder + required + '>' + value + '</textarea>';
    } else {
      control = '<input class="modal-field-control" type="text" name="' + name + '" value="' + value + '"' + placeholder + required + ' />';
    }
    return '<label class="modal-field" data-name="' + name + '">' +
      '<span class="modal-field-label">' + label + '</span>' +
      control +
      hint +
      '<div class="modal-field-error" hidden></div>' +
    '</label>';
  }

  function open(html) {
    var overlay = ensureOverlay();
    overlay.innerHTML = html;
    overlay.setAttribute('aria-hidden', 'false');
    overlay.classList.add('open');
    // Focus the first focusable control inside so keyboard users land
    // in the dialog, not stranded on the page behind.
    var focusable = overlay.querySelector('input, textarea, button.modal-confirm');
    if (focusable) setTimeout(function () { focusable.focus(); }, 0);

    keydownHandler = function (e) {
      if (e.key === 'Escape') close();
    };
    document.addEventListener('keydown', keydownHandler);
  }

  function close() {
    if (!overlayEl) return;
    overlayEl.classList.remove('open');
    overlayEl.setAttribute('aria-hidden', 'true');
    overlayEl.innerHTML = '';
    if (keydownHandler) {
      document.removeEventListener('keydown', keydownHandler);
      keydownHandler = null;
    }
    if (closeCallback) {
      var cb = closeCallback;
      closeCallback = null;
      cb();
    }
  }

  function confirm(opts) {
    opts = opts || {};
    var title = escapeHtml(opts.title || 'Are you sure?');
    var message = opts.message ? '<p class="modal-message">' + escapeHtml(opts.message) + '</p>' : '';
    var confirmLabel = escapeHtml(opts.confirmLabel || 'Confirm');
    var dangerCls = opts.danger ? ' modal-confirm-danger' : '';
    open(
      '<div class="modal-dialog" role="dialog" aria-modal="true">' +
        '<div class="modal-header">' +
          '<h2 class="modal-title">' + title + '</h2>' +
          '<button type="button" class="modal-close" aria-label="Close">&times;</button>' +
        '</div>' +
        '<div class="modal-body">' + message + '</div>' +
        '<div class="modal-footer">' +
          '<button type="button" class="modal-btn modal-cancel">Cancel</button>' +
          '<button type="button" class="modal-btn modal-confirm' + dangerCls + '">' + confirmLabel + '</button>' +
        '</div>' +
      '</div>'
    );

    overlayEl.querySelector('.modal-close').addEventListener('click', close);
    overlayEl.querySelector('.modal-cancel').addEventListener('click', close);
    overlayEl.querySelector('.modal-confirm').addEventListener('click', function () {
      close();
      if (typeof opts.onConfirm === 'function') opts.onConfirm();
    });
  }

  function form(opts) {
    opts = opts || {};
    var title = escapeHtml(opts.title || 'Form');
    var fields = Array.isArray(opts.fields) ? opts.fields : [];
    var fieldsHtml = fields.map(renderFieldHtml).join('');
    var submitLabel = escapeHtml(opts.submitLabel || 'Submit');
    var dangerCls = opts.danger ? ' modal-confirm-danger' : '';
    // Optional prominent description at the top of the body — used by
    // high-severity actions (Kill) to put the warning text where users
    // actually read it, instead of burying it in a field hint.
    var descHtml = opts.description
      ? '<p class="modal-message' + (opts.danger ? ' modal-message-danger' : '') + '">' +
          escapeHtml(opts.description) +
        '</p>'
      : '';

    open(
      '<form class="modal-dialog" role="dialog" aria-modal="true">' +
        '<div class="modal-header">' +
          '<h2 class="modal-title">' + title + '</h2>' +
          '<button type="button" class="modal-close" aria-label="Close">&times;</button>' +
        '</div>' +
        '<div class="modal-body modal-form-body">' +
          descHtml +
          fieldsHtml +
        '</div>' +
        '<div class="modal-footer">' +
          '<button type="button" class="modal-btn modal-cancel">Cancel</button>' +
          '<button type="submit" class="modal-btn modal-confirm' + dangerCls + '">' + submitLabel + '</button>' +
        '</div>' +
      '</form>'
    );

    overlayEl.querySelector('.modal-close').addEventListener('click', close);
    overlayEl.querySelector('.modal-cancel').addEventListener('click', close);
    overlayEl.querySelector('form').addEventListener('submit', function (e) {
      e.preventDefault();
      // Clear prior errors.
      var errs = overlayEl.querySelectorAll('.modal-field-error');
      for (var i = 0; i < errs.length; i++) {
        errs[i].hidden = true;
        errs[i].textContent = '';
      }
      var values = {};
      var failed = false;
      for (var f = 0; f < fields.length; f++) {
        var field = fields[f];
        var input = overlayEl.querySelector('[name="' + field.name + '"]');
        if (!input) continue;
        var raw = input.value;
        if (field.required && (raw == null || String(raw).trim() === '')) {
          showFieldError(field.name, 'Required');
          failed = true;
          continue;
        }
        if (field.type === 'json' && String(raw).trim() !== '') {
          try {
            values[field.name] = JSON.parse(raw);
          } catch (err) {
            showFieldError(field.name, 'Invalid JSON: ' + err.message);
            failed = true;
            continue;
          }
        } else {
          values[field.name] = raw;
        }
      }
      if (failed) return;
      close();
      if (typeof opts.onSubmit === 'function') opts.onSubmit(values);
    });
  }

  function showFieldError(name, msg) {
    var fieldEl = overlayEl.querySelector('.modal-field[data-name="' + name + '"]');
    if (!fieldEl) return;
    var errEl = fieldEl.querySelector('.modal-field-error');
    if (errEl) {
      errEl.textContent = msg;
      errEl.hidden = false;
    }
  }

  return {
    confirm: confirm,
    form: form,
    close: close,
  };
})();
