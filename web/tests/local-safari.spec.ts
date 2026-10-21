import { test, expect } from '@playwright/test';
import { mkdir, writeFile } from 'node:fs/promises';
import { control } from './session';

test('WEB-LOCAL-SAFARI records Secure cookie acceptance after real HTTP localhost enrollment', async ({ page, request, browserName }, info) => {
  expect(browserName).toBe('webkit');
  const credential = await control(request, '/local-credential');
  await page.goto('/');
  const result = await page.evaluate(async value => {
    const finish = await fetch('/web/v1/enroll/finish', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(value) });
    const session = await fetch('/web/v1/session');
    return { finish_status: finish.status, session_status: session.status, secure_cookie_sent: session.status === 200 };
  }, credential);
  expect(result.finish_status).toBe(200);
  expect([200, 401]).toContain(result.session_status);
  info.annotations.push({ type: 'localhost_secure_cookie', description: JSON.stringify(result) });
  await mkdir('../lab/evidence/web/metrics', { recursive: true });
  await writeFile(`../lab/evidence/web/metrics/WEB-LOCAL-SAFARI-${info.project.name}.json`, JSON.stringify(result) + '\n');
});
