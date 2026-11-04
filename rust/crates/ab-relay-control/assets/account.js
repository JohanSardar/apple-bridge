'use strict';
const el = id => document.getElementById(id);
const say = (text, kind) => { const s = el('status'); if (s) { s.textContent = text; s.className = 'status small' + (kind ? ' ' + kind : ''); } };
async function call(path, body) {
  const response = await fetch(path, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body ?? {}) });
  const data = await response.json().catch(() => ({}));
  if (!response.ok) { const error = new Error(data.message || data.error || 'That did not work. Try again.'); error.status = response.status; throw error; }
  return data;
}
const auth = (path, body) => call('/api/auth' + path, body);
const api = (action, body) => call('/v1/' + action, body);
const fields = form => Object.fromEntries(new FormData(form));
function guard(work) {
  return async event => {
    event?.preventDefault();
    say('');
    try { await work(event); } catch (error) {
      if (error.status === 401 && location.pathname !== '/sign-in' && location.pathname !== '/invite') { location.assign('/sign-in?next=' + encodeURIComponent(location.pathname + location.search)); return; }
      say(error.message, 'error');
    }
  };
}
function nextPage() {
  const next = new URLSearchParams(location.search).get('next');
  return next && next.startsWith('/') && !next.startsWith('//') ? next : '/';
}
el('sign-in')?.addEventListener('submit', guard(async event => {
  await auth('/sign-in/email', fields(event.target));
  location.assign(nextPage());
}));
el('passkey-sign-in')?.addEventListener('click', guard(async () => {
  const options = await (await fetch('/api/auth/passkey/generate-authenticate-options')).json();
  const credential = await navigator.credentials.get({ publicKey: PublicKeyCredential.parseRequestOptionsFromJSON(options) });
  await auth('/passkey/verify-authentication', { response: credential.toJSON() });
  location.assign(nextPage());
}));
el('add-passkey')?.addEventListener('click', guard(async () => {
  const reply = await fetch('/api/auth/passkey/generate-register-options');
  if (!reply.ok) { const error = new Error('Sign in again to add a passkey.'); error.status = reply.status; throw error; }
  const credential = await navigator.credentials.create({ publicKey: PublicKeyCredential.parseCreationOptionsFromJSON(await reply.json()) });
  await auth('/passkey/verify-registration', { response: credential.toJSON(), name: 'Passkey' });
  say('Passkey added. You can sign in with it now.', 'done');
}));
el('sign-out')?.addEventListener('click', guard(async () => { await auth('/sign-out'); location.assign('/sign-in'); }));
if (el('sign-up')) {
  const token = location.hash.slice(1);
  history.replaceState(null, '', location.pathname);
  (token ? auth('/invite/check', { token }) : Promise.reject(new Error('This page needs the full link from your invite.'))).then(() => {
    el('sign-up').hidden = false;
    say('');
  }).catch(error => say(error.message, 'error'));
  el('sign-up').addEventListener('submit', guard(async event => {
    await auth('/sign-up/email', fields(event.target));
    location.assign('/');
  }));
}
let grant;
el('device-lookup')?.addEventListener('submit', guard(async event => {
  grant = await api('grant-view', { code: fields(event.target).code });
  el('device-name').textContent = grant.name;
  el('device-fingerprint').textContent = grant.fingerprint;
  el('device-confirm').hidden = false;
}));
el('device-allow')?.addEventListener('click', guard(async () => {
  await api('confirm', { code: grant.code, fingerprint: grant.fingerprint });
  el('device-confirm').hidden = true;
  el('device-lookup').hidden = true;
  el('device-done').hidden = false;
}));
if (el('device-lookup') && new URLSearchParams(location.search).get('code')) el('device-lookup').requestSubmit();
async function loadDevices() {
  const { devices, limit } = await api('list');
  const root = el('devices');
  root.replaceChildren();
  if (!devices.length) {
    const none = document.createElement('p');
    none.className = 'muted';
    none.textContent = 'No Macs yet. On your Mac, run: bridge remote on';
    root.append(none);
    return;
  }
  for (const device of devices) {
    const item = document.createElement('article');
    item.className = 'item';
    const head = document.createElement('div');
    head.className = 'item-head';
    const name = document.createElement('h2');
    name.textContent = device.name;
    const state = document.createElement('span');
    state.className = 'tag ' + (device.state === 'active' ? 'good' : 'bad');
    state.textContent = device.state === 'active' ? 'Active' : 'Suspended';
    head.append(name, state);
    const address = document.createElement('p');
    address.className = 'muted small';
    address.textContent = 'https://' + device.hostname + '/mcp';
    const actions = document.createElement('div');
    actions.className = 'actions end';
    const act = (label, action, extra) => {
      const button = document.createElement('button');
      button.type = 'button';
      button.textContent = label;
      if (extra) button.className = extra;
      button.addEventListener('click', guard(async () => {
        if (action === 'revoke' && !confirm('Remove this Mac? Its address stops working for good.')) return;
        await api(action, { hostname: device.hostname });
        await loadDevices();
      }));
      return button;
    };
    actions.append(device.state === 'active' ? act('Suspend', 'suspend') : act('Resume', 'resume'), act('Remove', 'revoke', 'danger'));
    item.append(head, address, actions);
    root.append(item);
  }
  const room = document.createElement('p');
  room.className = 'muted small';
  room.textContent = `${devices.length} of ${limit} Macs`;
  root.append(room);
}
if (el('devices')) guard(loadDevices)();
