import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { DatabaseSync } from 'node:sqlite';
import { enroll, authenticate } from './domain-fixture.ts';

async function port() {
  const server = createServer();
  await new Promise<void>(done => server.listen(0, '127.0.0.1', done));
  const number = (server.address() as any).port;
  await new Promise<void>(done => server.close(() => done()));
  return number;
}
const cookies = (response: Response) => response.headers.getSetCookie().map(value => value.split(';')[0]).join('; ');

test('two CLI services start concurrently and retain one registration authority', { skip: !process.env.AB_TEST_BUNDLE }, async () => {
  const dir = mkdtempSync(join(tmpdir(), 'ab-process-domain-'));
  const databasePath = join(dir, 'accounts.sqlite');
  const secret = join(dir, 'secret');
  writeFileSync(secret, randomBytes(48).toString('base64url'), { mode: 0o600 });
  const common = { ...process.env, AB_ACCOUNTS_DB: databasePath, AB_ACCOUNTS_SECRET_FILE: secret, AB_MAX_ACCOUNTS: '5' };
  const children: ReturnType<typeof spawn>[] = [];
  const diagnostics: string[] = [];
  async function start(origin: string, flags: Record<string, string>) {
    const number = await port();
    const child = spawn(process.execPath, [process.env.AB_TEST_BUNDLE!, 'serve'], { env: { ...common, AB_PUBLIC_URL: origin, AB_ACCOUNTS_LISTEN: `127.0.0.1:${number}`, ...flags }, stdio: ['ignore', 'pipe', 'pipe'] });
    children.push(child);
    await new Promise<void>((done, reject) => {
      const timer = setTimeout(() => reject(Error('service readiness timeout')), 15000);
      let output = '';
      child.stderr!.on('data', data => { output += data; diagnostics.push(String(data)); });
      child.stdout!.on('data', data => { if (String(data).includes('accounts listening')) { clearTimeout(timer); done(); } });
      child.once('exit', code => { clearTimeout(timer); reject(Error(`service exited ${code}: ${output}`)); });
    });
    return async (path: string, body?: any, cookie = '') => fetch(`http://127.0.0.1:${number}/api/auth${path}`, { method: body === undefined ? 'GET' : 'POST', headers: { origin, cookie, 'content-type': 'application/json', 'x-forwarded-for': '198.51.100.12' }, ...(body === undefined ? {} : { body: JSON.stringify(body) }) });
  }
  try {
    const [oldHost, newHost] = await Promise.all([
      start('https://app.old.example', { AB_ALLOW_REGISTRATION: 'true', AB_REGISTRATION_ORIGIN: '' }),
      start('https://app.new.example', { AB_ALLOW_REGISTRATION: 'true', AB_REGISTRATION_ORIGIN: 'https://app.old.example' }),
    ]);
    const invite = execFileSync(process.execPath, [process.env.AB_TEST_BUNDLE!, 'invite', 'create', '--uses', '3'], { env: { ...common, AB_PUBLIC_URL: 'https://app.old.example' } }).toString().trim();
    const token = new URL(invite).hash.slice(1);
    const checked = await oldHost('/invite/check', { token });
    const invitation = cookies(checked);
    const signup = { email: 'process-fixture@example.test', name: '[ab-test] process', password: 'fixture password long enough' };
    assert.equal((await newHost('/sign-up/email', signup, invitation)).status, 403);
    assert.equal((await oldHost('/sign-up/email', signup, invitation)).status, 200);
    const responses = await Promise.all(Array.from({ length: 8 }, (_, index) => (index % 2 ? oldHost : newHost)('/sign-in/email', signup)));
    for (const response of responses) assert.equal(response.status, 200);
    assert.notEqual((await newHost('/sign-in/email', { ...signup, password: 'incorrect' })).status, 200);
    const adapter = host => ({ auth: { handler: async (request: Request) => {
      const path = new URL(request.url).pathname.replace('/api/auth', '') + new URL(request.url).search;
      const response = await host(path, request.method === 'GET' ? undefined : await request.json(), request.headers.get('cookie') ?? '');
      assert.ok((await response.clone().text()).length > 0, `empty response ${path}: ${response.status}: ${diagnostics.join('\n')}`);
      return response;
    } } });
    const [oldKey, newKey] = await Promise.all([
      enroll(adapter(oldHost) as any, 'https://app.old.example', cookies(responses[1])),
      enroll(adapter(newHost) as any, 'https://app.new.example', cookies(responses[0])),
    ]);
    const authenticated = await Promise.all([
      authenticate(adapter(oldHost) as any, 'https://app.old.example', oldKey),
      authenticate(adapter(newHost) as any, 'https://app.new.example', newKey),
    ]);
    for (const response of authenticated) assert.equal(response.status, 200);

    const defaultHost = await start('https://app.default.example', { AB_ALLOW_REGISTRATION: '', AB_REGISTRATION_ORIGIN: '' });
    assert.equal((await defaultHost('/sign-up/email', { ...signup, email: 'refused@example.test' }, invitation)).status, 403);
    const db = new DatabaseSync(databasePath);
    assert.equal((db.prepare('PRAGMA integrity_check').get() as any).integrity_check, 'ok');
    assert.equal((db.prepare('SELECT count(*) AS n FROM user').get() as any).n, 1);
    assert.equal((db.prepare('SELECT uses_left FROM invite').get() as any).uses_left, 2);
    assert.ok((db.prepare('SELECT count(*) AS n FROM session').get() as any).n >= 8);
    assert.equal((db.prepare('SELECT count(*) AS n FROM passkey').get() as any).n, 2);
    db.close();
  } finally {
    await Promise.all(children.map(child => new Promise<void>(done => { if (child.exitCode !== null || child.signalCode !== null) { done(); return; } child.once('exit', () => done()); child.kill('SIGTERM'); })));
    rmSync(dir, { recursive: true, force: true });
  }
});
