const panel = document.querySelector('#approval');
const passkeyLink = document.querySelector('#use-passkey');
const passkey = document.querySelector('#passkey');
passkeyLink?.addEventListener('click', () => {
  passkey.hidden = !passkey.hidden;
  passkeyLink.textContent = passkey.hidden ? 'Use a passkey instead' : 'Hide passkey';
});
const title = document.querySelector('#approval-title');
const statusLine = document.querySelector('#approval-status');
const match = document.querySelector('#approval-match');
const say = (heading, text, kind) => {
  if (heading) title.textContent = heading;
  statusLine.textContent = text;
  panel.className = 'next' + (kind ? ' ' + kind : '');
};
const wait = ms => new Promise(done => setTimeout(done, ms));
const reveal = (code, needed) => {
  if (!needed) return;
  document.querySelector('#approval-code').textContent = code;
  match.hidden = false;
};
async function requestApproval() {
  const created = await fetch('/oauth/pending' + location.search, { method: 'POST' });
  const request = await created.json();
  if (!created.ok) throw new Error(request.error?.message || 'Approval is unavailable. Try again in a few minutes.');
  reveal(request.code, request.show_code);
  const deadline = Date.parse(request.expires_at);
  while (Date.now() < deadline) {
    await wait(1200);
    let polled;
    try { polled = await fetch('/oauth/pending/' + encodeURIComponent(request.id)); } catch { continue; }
    if (polled.status === 429) { await wait(2000); continue; }
    const result = await polled.json();
    if (!polled.ok) throw new Error(result.error?.message || 'This approval has expired. Start again from your app.');
    reveal(request.code, result.show_code);
    if (result.redirect) {
      match.hidden = true;
      const denied = new URL(result.redirect).searchParams.has('error');
      const name = panel.dataset.client || 'the app';
      say(denied ? 'Not allowed.' : 'Allowed.', denied ? name + ' didn’t get access. Going back…' : 'Going back to ' + name + '…', denied ? 'stopped' : 'done');
      await wait(900);
      location.assign(result.redirect);
      return;
    }
  }
  throw new Error('This approval has expired. Start again from your app.');
}
if (panel) requestApproval().catch(error => {
  match.hidden = true;
  say('This didn’t go through.', error.message, 'stopped');
});
