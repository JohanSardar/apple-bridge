import { test, expect } from '@playwright/test';
import { mkdir, writeFile } from 'node:fs/promises';
import { control } from './session';

test('WEB-LOCAL-SAFARI enrolls over HTTP localhost and opens a live session', async ({ page, request, browserName }, info) => {
  expect(browserName).toBe('webkit');
  const credential = await control(request, '/local-credential');
  await page.goto('/');
  const result = await page.evaluate(async value => {
    const finish = await fetch('/web/v1/enroll/finish', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(value) });
    const session = await fetch('/web/v1/session');
    return { finish_status: finish.status, session_status: session.status };
  }, credential);
  expect(result.finish_status).toBe(200);
  expect(result.session_status).toBe(200);
  const cookie = (await page.context().cookies()).find(value => value.name === 'ab_session_local');
  expect(cookie).toMatchObject({ secure: false, httpOnly: true, sameSite: 'Strict', path: '/' });
  const welcome = await page.evaluate(() => new Promise<string>((resolve, reject) => {
    const socket = new WebSocket(location.origin.replace('http:', 'ws:') + '/web/v1/live', 'web.1');
    socket.onopen = () => socket.send(JSON.stringify({ t: 'hello', v: 1 }));
    socket.onmessage = event => { const frame = JSON.parse(event.data); if (frame.t === 'welcome') socket.send(JSON.stringify({ t: 'ping' })); else { resolve(frame.t); socket.close(); } };
    socket.onerror = () => reject(new Error('live connection failed'));
  }));
  expect(welcome).toBe('pong');
  const boundaries = await request.post(process.env.AB_WEB_CONTROL! + '/cookie-boundaries', {
    headers: { 'X-Bridge-Lab': process.env.AB_WEB_CONTROL_KEY! }, data: { token: cookie!.value }
  });
  expect(boundaries.ok()).toBe(true);
  expect(await boundaries.json()).toEqual({ public_local: 401, tls_local: 401, http_host: 401, http_local: 200, public_host: 200 });
  expect(await page.evaluate(async () => (await fetch('/web/v1/logout', { method: 'POST' })).status)).toBe(200);
  expect((await page.context().cookies()).some(value => value.name === 'ab_session_local')).toBe(false);
  expect(await page.evaluate(async () => (await fetch('/web/v1/session')).status)).toBe(401);
  await mkdir('../lab/evidence/web/metrics', { recursive: true });
  await writeFile(`../lab/evidence/web/metrics/WEB-LOCAL-SAFARI-${info.project.name}.json`, JSON.stringify({ ...result, live: welcome }) + '\n');
});
