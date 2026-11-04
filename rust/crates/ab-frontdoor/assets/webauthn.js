const redirect = document.querySelector("#connector-redirect");
if (redirect) location.assign(redirect.href);
const decode = s => Uint8Array.from(atob(s.replace(/-/g, '+').replace(/_/g, '/')), c => c.charCodeAt(0));
const encode = b => btoa(String.fromCharCode(...new Uint8Array(b))).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
async function post(path, body) {
 const response = await fetch(path, {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify(body)});
 const result = await response.json();
 if (!response.ok) throw new Error(result.error?.message || 'Sign in failed');
 return result;
}
document.querySelector('#signin')?.addEventListener('click', async () => {
 const message = document.querySelector('#message');
 try {
  const enrolling = location.pathname === '/enroll';
  const flow = enrolling ? 'enroll' : 'login';
  const token = location.hash.slice(1);
  const options = await post('/auth/' + flow + '/options', {token,kind:'trusted'});
  const publicKey = options.publicKey;
  publicKey.challenge = decode(publicKey.challenge);
  if (enrolling) publicKey.user.id = decode(publicKey.user.id);
  for (const key of ['allowCredentials','excludeCredentials']) publicKey[key]?.forEach(c => c.id = decode(c.id));
  const credential = await navigator.credentials[enrolling ? 'create' : 'get']({publicKey});
  const response = {};
  for (const key of ['clientDataJSON','attestationObject','authenticatorData','signature','userHandle']) if (credential.response[key]) response[key] = encode(credential.response[key]);
  if (credential.response.getTransports) response.transports = credential.response.getTransports();
  await post('/auth/' + flow + '/finish', {ceremony_id:options.ceremony_id,credential:{id:credential.id,rawId:encode(credential.rawId),type:credential.type,response,clientExtensionResults:credential.getClientExtensionResults()}});
  history.replaceState(null,'',location.pathname + location.search);
  message.textContent = 'Signed in. You may continue.';
  if (location.pathname.startsWith('/approvals') || location.pathname === '/oauth/authorize') { location.assign(location.href); return; }
  document.querySelector('#signin').hidden = true;
 } catch (error) { message.textContent = error.message; }
});
document.addEventListener('submit', async event => {
 const form = event.target;
 if (!(form instanceof HTMLFormElement)) return;
 event.preventDefault();
 try {
  const response = await fetch(form.action, {method:'POST',body:new URLSearchParams(new FormData(form,event.submitter))});
  const html = await response.text();
  if (!response.ok) throw new Error('Request refused: ' + response.status);
  const documentResult = new DOMParser().parseFromString(html,'text/html');
  const next = documentResult.querySelector('#connector-redirect');
  if (next) { location.assign(next.href); return; }
  document.querySelector('main').replaceWith(documentResult.querySelector('main'));
 } catch (error) {
  const message = document.querySelector('#message') || document.querySelector('main').appendChild(document.createElement('p'));
  message.textContent = error.message;
 }
});
