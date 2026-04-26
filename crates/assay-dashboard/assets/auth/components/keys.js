/* JWKS / Biscuit pane — view active signing material. */

var AssayAuthKeys = (function () {
  'use strict';

  let ctx = null;
  let container = null;

  function render(el, c) {
    ctx = c;
    container = el;
    container.innerHTML =
      '<div class="auth-toolbar">' +
        '<h2 class="section-title">JWKS / Biscuit Keys</h2>' +
        '<button type="button" class="btn" id="kk-refresh">Refresh</button>' +
      '</div>' +
      '<div id="kk-wrap"><div class="auth-empty">Loading…</div></div>';
    document.getElementById('kk-refresh').addEventListener('click', load);
    load();
  }

  async function load() {
    const wrap = container.querySelector('#kk-wrap');
    wrap.innerHTML = '<div class="auth-empty">Loading keys…</div>';
    try {
      const [biscuit, jwks] = await Promise.all([
        ctx.api.biscuit().catch(function (e) { return { error: e.message }; }),
        ctx.api.jwks().catch(function (e) { return { error: e.message }; }),
      ]);
      let html = '<div class="auth-pane-section"><h3>Biscuit root key</h3>';
      if (biscuit && !biscuit.error) {
        html += '<dl class="auth-form">' +
          '<dt>Active kid</dt><dd class="auth-mono">' + ctx.escapeHtml(biscuit.kid) + '</dd>' +
          '<dt>Public PEM</dt><dd><pre class="auth-tree">' + ctx.escapeHtml(biscuit.public_pem) + '</pre></dd>' +
          '</dl>';
      } else {
        html += '<p class="auth-empty">' + ctx.escapeHtml((biscuit && biscuit.error) || 'unavailable') + '</p>';
      }
      html += '</div>';

      html += '<div class="auth-pane-section"><h3>JWKS document</h3>';
      if (jwks && !jwks.error) {
        html += '<pre class="auth-tree">' + ctx.escapeHtml(JSON.stringify(jwks, null, 2)) + '</pre>';
      } else {
        html += '<p class="auth-empty">' + ctx.escapeHtml((jwks && jwks.error) || 'unavailable') + '</p>';
      }
      html += '<p class="auth-empty">Rotation is performed via the engine boot path or operator tooling — phase 8b surfaces the active material read-only.</p>';
      html += '</div>';
      wrap.innerHTML = html;
    } catch (err) {
      wrap.innerHTML = '<div class="auth-empty">Error: ' + ctx.escapeHtml(err.message) + '</div>';
    }
  }

  if (typeof window !== 'undefined') {
    window.AssayAuthKeys = { render: render };
  }

  return { render: render };
})();
