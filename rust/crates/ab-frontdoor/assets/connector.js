const panel = document.querySelector('#approval');
const passkeyLink = document.querySelector('#use-passkey');
const passkey = document.querySelector('#passkey');
passkeyLink?.addEventListener('click', () => {
  passkey.hidden = !passkey.hidden;
  passkeyLink.textContent = passkey.hidden ? 'Use a passkey instead' : 'Hide passkey';
});
const statusLine = document.querySelector('#approval-status');
const say = (text, kind) => { statusLine.textContent = text; statusLine.className = 'status' + (kind ? ' ' + kind : ''); };
const wait = ms => new Promise(done => setTimeout(done, ms));
async function requestApproval() {
  const created = await fetch('/oauth/pending' + location.search, { method: 'POST' });
  const request = await created.json();
  if (!created.ok) throw new Error(request.error?.message || 'Approval is unavailable. Try again in a few minutes.');
  document.querySelector('#approval-code').textContent = request.code;
  say('Waiting for your approval…');
  const deadline = Date.parse(request.expires_at);
  while (Date.now() < deadline) {
    await wait(1500);
    let polled;
    try { polled = await fetch('/oauth/pending/' + encodeURIComponent(request.id)); } catch { continue; }
    if (polled.status === 429) { await wait(2000); continue; }
    const result = await polled.json();
    if (!polled.ok) throw new Error(result.error?.message || 'This approval has expired. Start again from your app.');
    if (result.redirect) {
      say(new URL(result.redirect).searchParams.has('error') ? 'Denied. Returning to the app…' : 'Approved. Returning to the app…', 'done');
      location.assign(result.redirect);
      return;
    }
  }
  throw new Error('This approval has expired. Start again from your app.');
}
if (panel) requestApproval().catch(error => {
  document.querySelector('#approval-code').hidden = true;
  say(error.message, 'error');
});
