// auth.js — authentication state management
//
// On load:
//   1. Fetch /api/manifest  (unauthenticated, cacheable) to learn which
//      providers are configured and what the quota limits are.
//   2. If auth is enabled, fetch /auth/me to learn the current user state.
//   3. Render the sign-in button (only if auth.enabled), provider list,
//      and quota indicators.

// ── Globals ──────────────────────────────────────────────────────────────────

// Exposed on window so other scripts can read quota limits.
window.lwManifest = null;

// ── Modal ─────────────────────────────────────────────────────────────────────

const authModal = (() => {
  const el = document.getElementById('auth-modal');
  return {
    open()  { el.style.display = 'flex'; },
    close() { el.style.display = 'none'; },
  };
})();

// ── Actions ───────────────────────────────────────────────────────────────────

async function authLogout() {
  await fetch('/auth/logout', { method: 'POST' });
  location.reload();
}

async function authSendMagicLink(e) {
  e.preventDefault();
  const email = e.target.email.value;
  const resp = await fetch('/auth/magic', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ email }),
  });
  if (resp.ok) {
    document.getElementById('auth-providers').innerHTML =
      '<p class="auth-magic-sent">Check your inbox for the sign-in link.<br><span class="auth-magic-spam">If you don\'t see it, check your spam folder.</span></p>';
  }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function renderProviders(providers) {
  const el = document.getElementById('auth-providers');
  if (!el) return;
  if (!providers || providers.length === 0) {
    el.innerHTML = '<p class="auth-no-providers">No sign-in methods configured.</p>';
    return;
  }
  el.innerHTML = providers.map(p => {
    if (p === 'github') return `
      <a href="/auth/github" class="btn btn--provider btn--github">
        <svg class="btn--provider__icon" viewBox="0 0 24 24" aria-hidden="true" fill="currentColor">
          <path d="M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23.96-.27 1.98-.405 3-.405s2.04.135 3 .405c2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.02 12.02 0 0 0 24 12c0-6.63-5.37-12-12-12z"/>
        </svg>
        Sign in with GitHub
      </a>`;
    if (p === 'google') return `
      <a href="/auth/google" class="btn btn--provider btn--google">
        <svg class="btn--provider__icon" viewBox="0 0 24 24" aria-hidden="true">
          <path fill="#4285F4" d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92c-.26 1.37-1.04 2.53-2.21 3.31v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.09z"/>
          <path fill="#34A853" d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z"/>
          <path fill="#FBBC05" d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l3.66-2.84z"/>
          <path fill="#EA4335" d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z"/>
        </svg>
        Sign in with Google
      </a>`;
    if (p === 'email')  return `
      <div class="auth-divider"><span>or continue with email</span></div>
      <form class="auth-magic-form" onsubmit="authSendMagicLink(event)">
        <input type="email" name="email" placeholder="Email address" required class="input" />
        <button type="submit" class="btn btn--primary">Send magic link</button>
      </form>`;
    return '';
  }).join('');
}

function renderSignedIn(me) {
  const signinBtn    = document.getElementById('auth-signin-btn');
  const signinWrap   = document.getElementById('signin-tooltip-wrap');
  const userMenu     = document.getElementById('auth-user-menu');
  const avatarBtn    = document.getElementById('auth-avatar-btn');
  const userInfo     = document.getElementById('auth-user-info');
  const dropdown     = document.getElementById('auth-dropdown');

  if (signinBtn)  signinBtn.style.display  = 'none';
  if (signinWrap) signinWrap.style.display = 'none';
  if (userMenu)  userMenu.style.display  = 'flex';

  if (avatarBtn) {
    const initials = (me.display_name || me.email || '?').slice(0, 2).toUpperCase();
    avatarBtn.textContent = initials;
    avatarBtn.title       = me.display_name || me.email || '';
    avatarBtn.onclick = () => {
      if (dropdown) dropdown.style.display = dropdown.style.display === 'none' ? 'block' : 'none';
    };
  }

  if (userInfo) {
    userInfo.innerHTML = `
      <div class="auth-user-name">${me.display_name || ''}</div>
      <div class="auth-user-email">${me.email || ''}</div>
      <div class="auth-user-tier">${me.tier}</div>`;
  }
}

// ── Boot ──────────────────────────────────────────────────────────────────────

async function initAuth() {
  // Step 1: fetch the server manifest (unauthenticated, always available).
  let manifest;
  try {
    const resp = await fetch('/api/manifest');
    if (!resp.ok) return;
    manifest = await resp.json();
  } catch { return; }

  window.lwManifest = manifest;

  // If auth is not configured, hide the sign-in button entirely and stop.
  const signinBtn  = document.getElementById('auth-signin-btn');
  const signinWrap = document.getElementById('signin-tooltip-wrap');
  if (!manifest.auth.enabled) {
    if (signinBtn)  signinBtn.style.display  = 'none';
    if (signinWrap) signinWrap.style.display = 'none';
    return;
  }

  // Auth is available — populate provider list in the modal.
  renderProviders(manifest.auth.providers);

  // Step 2: fetch current user state.
  let me;
  try {
    const resp = await fetch('/auth/me');
    if (!resp.ok) return;
    me = await resp.json();
  } catch { return; }

  // Step 3: if signed in, swap sign-in button for avatar menu.
  if (me.id && me.id !== '') {
    renderSignedIn(me);
  }
  // Otherwise the sign-in button remains visible (default state in HTML).
}

document.addEventListener('DOMContentLoaded', initAuth);
