// Login landing — fetch enabled upstreams from /auth/upstreams and
// render one button per row. Preserves the `return_to` query
// parameter so the user lands back at the page that bounced them
// here once the upstream round-trip completes.

(function () {
  var params = new URLSearchParams(window.location.search);
  var returnTo = params.get('return_to') || '/';

  var container = document.getElementById('upstreams');
  if (!container) return;

  function showStatus(text, isError) {
    container.dataset.state = isError ? 'error' : 'empty';
    container.innerHTML = '';
    var p = document.createElement('p');
    p.className = 'login-status' + (isError ? ' login-status-error' : '');
    p.textContent = text;
    container.appendChild(p);
  }

  fetch('/auth/upstreams', { credentials: 'omit' })
    .then(function (r) {
      if (!r.ok) throw new Error('http ' + r.status);
      return r.json();
    })
    .then(function (upstreams) {
      if (!Array.isArray(upstreams) || upstreams.length === 0) {
        showStatus('No sign-in providers configured.');
        return;
      }
      container.dataset.state = 'ready';
      container.innerHTML = '';
      upstreams.forEach(function (u) {
        var a = document.createElement('a');
        a.className = 'login-button';
        a.href = '/auth/oidc/upstream/' + encodeURIComponent(u.slug)
          + '/start?return_to=' + encodeURIComponent(returnTo);
        a.dataset.slug = u.slug;

        if (u.icon_url) {
          var img = document.createElement('img');
          img.src = u.icon_url;
          img.alt = '';
          img.width = 20;
          img.height = 20;
          a.appendChild(img);
        }
        var label = document.createElement('span');
        label.textContent = 'Sign in with ' + (u.display_name || u.slug);
        a.appendChild(label);
        container.appendChild(a);
      });
    })
    .catch(function () {
      showStatus('Could not load sign-in options.', true);
    });
})();
