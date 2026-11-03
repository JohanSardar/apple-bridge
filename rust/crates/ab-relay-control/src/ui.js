'use strict';
let csrf = document.querySelector('meta[name=csrf]')?.content || sessionStorage.getItem('csrf') || '';
if (csrf) sessionStorage.setItem('csrf', csrf);
const accountMeta = document.querySelector('meta[name=account]')?.content;
if (accountMeta) sessionStorage.setItem('account', accountMeta);
let confirmation;
const el = id => document.getElementById(id);
const decode = value => Uint8Array.from(atob(value.replace(/-/g, '+').replace(/_/g, '/')), c => c.charCodeAt(0));
const encode = value => btoa(String.fromCharCode(...new Uint8Array(value))).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
async function api(action, input = {}) {
  const response = await fetch('/v1/' + action, {method: 'POST', headers: {'content-type': 'application/json', 'x-csrf-token': csrf}, body: JSON.stringify(input)});
  const data = await response.json();
  if (!response.ok) throw new Error(data.error || 'Request failed');
  if (data.csrf) {csrf = data.csrf; sessionStorage.setItem('csrf', csrf); sessionStorage.setItem('account', data.account);}
  return data;
}
async function run(fn) {
  try {el('status').textContent = ''; await fn();} catch (error) {el('status').textContent = error.message;}
}
function form(id, fn) {el(id).addEventListener('submit', event => {event.preventDefault(); run(() => fn(Object.fromEntries(new FormData(event.target))));});}
function options(data) {
  const pk = data.publicKey;
  pk.challenge = decode(pk.challenge);
  if (pk.user) pk.user.id = decode(pk.user.id);
  for (const field of ['allowCredentials', 'excludeCredentials']) if (pk[field]) pk[field] = pk[field].map(c => ({...c, id: decode(c.id)}));
  return {publicKey: pk};
}
function credential(input) {
  const response = {};
  for (const field of ['clientDataJSON', 'attestationObject', 'authenticatorData', 'signature']) if (input.response[field]) response[field] = encode(input.response[field]);
  return {id: input.id, response};
}
form('github', async data => {const result = await api('oauth-start', data); location.assign(result.url);});
form('login', async data => {
  const start = await api('passkey-login-start', data);
  const key = await navigator.credentials.get(options(start));
  await api('passkey-login-finish', {...credential(key), transaction: start.transaction});
  el('status').textContent = 'Signed in';
});
form('recovery', async data => {await api('recover', data); el('status').textContent = 'Recovered. Add a new service passkey.';});
el('register').onclick = () => run(async () => {
  const start = await api('passkey-register-start');
  const key = await navigator.credentials.create(options(start));
  const result = await api('passkey-register-finish', {...credential(key), transaction: start.transaction});
  el('codes').textContent = 'Save these single-use recovery codes securely:\n' + result.recovery_codes.join('\n');
});
form('lookup', async data => {
  confirmation = await api('confirm-view', data);
  el('details').textContent = 'Code: ' + confirmation.code + '\nFingerprint: ' + confirmation.fingerprint + '\nSlot: ' + confirmation.slot;
  el('confirmation').hidden = false;
});
el('confirm').onclick = () => run(async () => {await api('confirm', confirmation); el('confirmation').hidden = true; el('status').textContent = 'Approved. Complete enrollment on your Mac.';});
el('list').onclick = () => run(async () => {
  const data = await api('list'); el('devices').textContent = JSON.stringify(data, null, 2); el('actions').replaceChildren();
  for (const device of data.devices) for (const action of ['suspend', 'resume', 'revoke', 'rotate']) {
    const button = document.createElement('button'); button.textContent = action + ' ' + device.id;
    button.onclick = () => run(async () => {
      if (action === 'rotate') {
        const result = await api('rotate-start', {device: device.id});
        el('rotate').hidden = false; el('rotate').elements.device.value = device.id; el('rotate').elements.nonce.value = result.nonce;
        el('status').textContent = 'Sign this rotation nonce with the new key on your Mac: ' + result.nonce;
      } else {await api(action, {device: device.id}); el('list').click();}
    }); el('actions').append(button);
  }
});

form('rotate', async data => {const result = await api('rotate', data); el('codes').textContent = 'Save the new Mac tunnel credential securely: ' + result.credential; el('rotate').hidden = true;});
