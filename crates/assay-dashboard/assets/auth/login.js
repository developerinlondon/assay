/* Assay Auth — login landing controller.
 *
 * Submits first-party email/password credentials to the engine session
 * endpoint. Enabled upstream IdPs are rendered as optional alternatives.
 *
 * Provider icons come from /auth/icons.svg (a single sprite shipped
 * with the auth dashboard). The button references the right symbol by
 * slug — Google/GitHub/GitLab/Microsoft/Apple/Discord/Slack are mapped
 * to their Simple Icons paths; everything else falls back to a generic
 * shield-and-person glyph. Operators can override per-upstream by
 * setting icon_url on the upstream row.
 *
 * DOM-build only — never innerHTML with provider-supplied fields
 * (display_name, icon_url). textContent + element properties are
 * XSS-safe by construction.
 */

(function () {
  'use strict';

  const params = new URLSearchParams(window.location.search);
  const returnTo = safeReturnTo(params.get('return_to'));

  const container = document.getElementById('upstreams');
  const upstreamSection = document.getElementById('upstream-login');
  const upstreamStatus = document.getElementById('upstream-status');
  const passwordForm = document.getElementById('password-login');
  const emailInput = document.getElementById('email');
  const passwordInput = document.getElementById('password');
  const passwordError = document.getElementById('password-error');
  const passwordSubmit = document.getElementById('password-submit');

  function safeReturnTo(raw) {
    try {
      const candidate = new URL(raw || '/', window.location.origin);
      if (candidate.origin !== window.location.origin) return '/';
      return candidate.pathname + candidate.search + candidate.hash;
    } catch (_error) {
      return '/';
    }
  }

  function showPasswordError(message) {
    passwordError.textContent = message;
    passwordInput.value = '';
    passwordInput.type = 'password';
    const reveal = document.getElementById('password-reveal');
    if (reveal) {
      reveal.setAttribute('aria-pressed', 'false');
      const label = reveal.querySelector('.login-reveal-label');
      if (label) label.textContent = 'Show';
    }
    passwordInput.focus();
  }

  function submitPasswordLogin(event) {
    event.preventDefault();
    passwordError.textContent = '';
    passwordSubmit.disabled = true;
    passwordSubmit.dataset.idleLabel = passwordSubmit.textContent;
    passwordSubmit.textContent = 'Signing in\u2026';
    fetch('/api/v1/engine/auth/login', {
      method: 'POST',
      credentials: 'same-origin',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        email: emailInput.value,
        password: passwordInput.value
      })
    }).then(function (response) {
      if (!response.ok) throw new Error('invalid credentials');
      window.location.assign(returnTo);
    }).catch(function () {
      showPasswordError('Email or password is incorrect.');
      if (passwordSubmit.dataset.idleLabel) {
        passwordSubmit.textContent = passwordSubmit.dataset.idleLabel;
      }
      passwordSubmit.disabled = false;
    });
  }

  if (passwordForm) passwordForm.addEventListener('submit', submitPasswordLogin);

  // Reveal control. Additive and self-contained — it only ever flips the
  // input's type, so a browser that never runs this block still has a
  // working password field.
  const passwordReveal = document.getElementById('password-reveal');
  if (passwordReveal && passwordInput) {
    passwordReveal.addEventListener('click', function () {
      const shown = passwordInput.type === 'text';
      passwordInput.type = shown ? 'password' : 'text';
      passwordReveal.setAttribute('aria-pressed', shown ? 'false' : 'true');
      passwordReveal.title = shown ? 'Show password' : 'Hide password';
      const label = passwordReveal.querySelector('.login-reveal-label');
      if (label) label.textContent = shown ? 'Show' : 'Hide';
      passwordInput.focus();
    });
  }
  if (!container || !upstreamSection || !upstreamStatus) return;

  const SVG_NS = 'http://www.w3.org/2000/svg';
  const XLINK_NS = 'http://www.w3.org/1999/xlink';

  // Map an upstream to a sprite symbol id. Substring match on slug +
  // display_name so a provider named "Corporate Google Workspace" still
  // gets the Google icon. Returns 'generic' for anything unmatched.
  function spriteIdFor(slug, displayName) {
    const key = ((slug || '') + ' ' + (displayName || '')).toLowerCase();
    if (key.indexOf('google') !== -1) return 'google';
    if (key.indexOf('github') !== -1) return 'github';
    if (key.indexOf('gitlab') !== -1) return 'gitlab';
    if (key.indexOf('microsoft') !== -1 || key.indexOf('azure') !== -1 || key.indexOf('entra') !== -1) return 'microsoft';
    if (key.indexOf('apple') !== -1) return 'apple';
    if (key.indexOf('discord') !== -1) return 'discord';
    if (key.indexOf('slack') !== -1) return 'slack';
    return 'generic';
  }

  function makeSpriteIcon(symbolId) {
    const svg = document.createElementNS(SVG_NS, 'svg');
    svg.setAttribute('width', '20');
    svg.setAttribute('height', '20');
    svg.setAttribute('aria-hidden', 'true');
    const use = document.createElementNS(SVG_NS, 'use');
    // Set both `href` (modern) and `xlink:href` (legacy) — same-doc
    // references work without either on modern browsers, but external
    // sprite refs need `href` and some user agents only honour xlink.
    use.setAttribute('href', '/auth/icons.svg#' + symbolId);
    use.setAttributeNS(XLINK_NS, 'xlink:href', '/auth/icons.svg#' + symbolId);
    svg.appendChild(use);
    return svg;
  }

  // Brand-tinted hover class names. Same lookup as the sprite id.
  function brandClassFor(slug, displayName) {
    const key = ((slug || '') + ' ' + (displayName || '')).toLowerCase();
    if (key.indexOf('google') !== -1) return 'is-google';
    if (key.indexOf('github') !== -1) return 'is-github';
    if (key.indexOf('gitlab') !== -1) return 'is-gitlab';
    if (key.indexOf('microsoft') !== -1 || key.indexOf('azure') !== -1 || key.indexOf('entra') !== -1) return 'is-microsoft';
    if (key.indexOf('apple') !== -1) return 'is-apple';
    if (key.indexOf('discord') !== -1) return 'is-discord';
    if (key.indexOf('slack') !== -1) return 'is-slack';
    return null;
  }

  function showUpstreamStatus(text, isError) {
    upstreamStatus.className = 'login-status' + (isError ? ' login-status-error' : '');
    upstreamStatus.textContent = text;
    upstreamStatus.hidden = !text;
  }

  // Same-origin fetch — must allow cookies so the browser attaches the
  // Cloudflare Access cookie (or any perimeter cookie) when this page
  // is loaded through such a gate. `credentials: 'omit'` would strip
  // those and the upstream call gets bounced to the CF Access login.
  fetch('/auth/upstreams', { credentials: 'same-origin' })
    .then(function (r) {
      if (!r.ok) throw new Error('http ' + r.status);
      return r.json();
    })
    .then(function (upstreams) {
      if (!Array.isArray(upstreams) || upstreams.length === 0) {
        showUpstreamStatus('', false);
        return;
      }
      upstreamSection.hidden = false;
      showUpstreamStatus('', false);
      container.innerHTML = '';
      upstreams.forEach(function (u) {
        const a = document.createElement('a');
        a.className = 'login-button';
        a.href = '/auth/oidc/upstream/' + encodeURIComponent(u.slug)
          + '/start?return_to=' + encodeURIComponent(returnTo);
        a.dataset.slug = u.slug;
        const brandClass = brandClassFor(u.slug, u.display_name);
        if (brandClass) a.classList.add(brandClass);

        // Icon resolution:
        //   1. Operator-configured icon_url (raster or SVG asset)
        //   2. Sprite symbol from /auth/icons.svg keyed on provider
        //      (falls back to the 'generic' symbol for unknowns)
        const iconWrap = document.createElement('span');
        iconWrap.className = 'login-button-icon';
        if (u.icon_url) {
          const img = document.createElement('img');
          // `no-referrer` keeps the Referer header off the icon
          // fetch — otherwise a third-party CDN hosting the icon
          // would see `/auth/login?return_to=...` (OIDC request
          // metadata) in its logs.
          img.referrerPolicy = 'no-referrer';
          img.src = u.icon_url;
          img.alt = '';
          img.width = 20;
          img.height = 20;
          iconWrap.appendChild(img);
        } else {
          iconWrap.appendChild(makeSpriteIcon(spriteIdFor(u.slug, u.display_name)));
        }
        a.appendChild(iconWrap);

        const label = document.createElement('span');
        label.className = 'login-button-label';
        label.textContent = 'Sign in with ' + (u.display_name || u.slug);
        a.appendChild(label);
        container.appendChild(a);
      });
    })
    .catch(function () {
      showUpstreamStatus('Could not load additional sign-in options.', true);
    });
})();
