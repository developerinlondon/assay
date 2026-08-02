(function () {
  'use strict';

  const fragment = new URLSearchParams(window.location.hash.slice(1));
  const token = fragment.get('token');
  window.history.replaceState(null, '', window.location.pathname + window.location.search);

  const requestForm = document.getElementById('recovery-request');
  const completeForm = document.getElementById('recovery-complete');
  const emailInput = document.getElementById('recovery-email');
  const requestStatus = document.getElementById('request-status');
  const requestSubmit = document.getElementById('request-submit');
  const passwordInput = document.getElementById('new-password');
  const confirmInput = document.getElementById('confirm-password');
  const completeStatus = document.getElementById('complete-status');
  const completeSubmit = document.getElementById('complete-submit');

  if (token) {
    requestForm.hidden = true;
    completeForm.hidden = false;
    passwordInput.focus();
  }

  requestForm.addEventListener('submit', function (event) {
    event.preventDefault();
    requestSubmit.disabled = true;
    requestStatus.className = 'login-status';
    requestStatus.textContent = '';
    fetch('/api/v1/engine/auth/password/recovery/request', {
      method: 'POST',
      credentials: 'same-origin',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ email: emailInput.value })
    }).then(function (response) {
      if (!response.ok) throw new Error('recovery unavailable');
      requestStatus.textContent = 'If an account exists for that address, a reset link has been sent.';
      requestForm.reset();
    }).catch(function () {
      requestStatus.className = 'login-status login-status-error';
      requestStatus.textContent = 'Password recovery is temporarily unavailable.';
    }).finally(function () {
      requestSubmit.disabled = false;
    });
  });

  completeForm.addEventListener('submit', function (event) {
    event.preventDefault();
    completeStatus.textContent = '';
    if (passwordInput.value !== confirmInput.value) {
      completeStatus.textContent = 'Passwords do not match.';
      confirmInput.focus();
      return;
    }
    completeSubmit.disabled = true;
    fetch('/api/v1/engine/auth/password/recovery/complete', {
      method: 'POST',
      credentials: 'same-origin',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ token: token, password: passwordInput.value })
    }).then(function (response) {
      if (!response.ok) throw new Error('recovery rejected');
      window.location.assign('/auth/login');
    }).catch(function () {
      completeStatus.textContent = 'This reset link is invalid or has expired.';
      passwordInput.value = '';
      confirmInput.value = '';
      passwordInput.focus();
      completeSubmit.disabled = false;
    });
  });
})();
