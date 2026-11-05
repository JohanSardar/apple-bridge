import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { createServer, type Server } from 'node:http';
import type { AddressInfo } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { after, before, beforeEach, test } from 'node:test';
import vm from 'node:vm';
import { toNodeHandler } from 'better-auth/node';
import { INVITE_COOKIE, open, SESSION_COOKIE } from '../src/auth.ts';

const origin = 'https://app.example.test';
let directory: string;
let accounts: ReturnType<typeof open>;
let server: Server;
let base: string;
let client = 0;

before(async () => {
  directory = mkdtempSync(join(tmpdir(), 'ab-accounts-'));
  accounts = open({
    publicUrl: origin,
    databasePath: join(directory, 'accounts.sqlite'),
    secret: 'x'.repeat(40),
    maxAccounts: 5,
  });
  await accounts.migrate();
  const handle = toNodeHandler(accounts.auth);
  server = createServer((request, response) => void handle(request, response));
  await new Promise<void>((done) => server.listen(0, '127.0.0.1', () => done()));
  base = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
});
after(() => {
  server.close();
  rmSync(directory, { recursive: true, force: true });
});
beforeEach(() => {
  client += 1;
});

type Reply = { status: number; body: any; cookies: string[] };
async function post(path: string, body: unknown, options: { cookie?: string; origin?: string } = {}): Promise<Reply> {
  const response = await fetch(`${base}/api/auth${path}`, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      origin: options.origin ?? origin,
      'x-forwarded-for': `203.0.113.${client}`,
      ...(options.cookie ? { cookie: options.cookie } : {}),
    },
    body: JSON.stringify(body),
  });
  return { status: response.status, body: await response.json().catch(() => null), cookies: response.headers.getSetCookie() };
}
const pairs = (cookies: string[]) => cookies.map((cookie) => cookie.split(';')[0]).join('; ');
const signUp = (email: string, cookie?: string) =>
  post('/sign-up/email', { email, password: 'correct horse battery', name: email }, { cookie });
async function inviteCookie(token: string) {
  const reply = await post('/invite/check', { token });
  assert.equal(reply.status, 200, JSON.stringify(reply.body));
  return pairs(reply.cookies);
}
async function session(cookie: string) {
  const response = await fetch(`${base}/api/auth/get-session`, { headers: { cookie } });
  return response.json() as Promise<any>;
}

test('an account needs an invite', async () => {
  const reply = await signUp('nobody@example.test');
  assert.equal(reply.status, 403);
});

test('an invite admits exactly as many people as it says, then stops', async () => {
  const { token } = accounts.invites.create({ uses: 1, days: 1 });
  const cookie = await inviteCookie(token);
  const first = await signUp('first@example.test', cookie);
  assert.equal(first.status, 200, JSON.stringify(first.body));
  const again = await signUp('second@example.test', cookie);
  assert.equal(again.status, 403);
  assert.equal((await post('/invite/check', { token })).status, 403);
});

test('a forged invite cookie is refused', async () => {
  const { token } = accounts.invites.create({ uses: 1, days: 1 });
  const reply = await signUp('forged@example.test', `${INVITE_COOKIE}=${token}`);
  assert.equal(reply.status, 403);
  assert.equal(accounts.invites.valid(token), true, 'the invite was not spent');
});

test('expired and revoked invites are refused', async () => {
  const old = accounts.invites.create({ uses: 1, days: 1 }, Date.now() - 2 * 86_400_000);
  assert.equal((await post('/invite/check', { token: old.token })).status, 403);
  const revoked = accounts.invites.create({ uses: 3, days: 1 });
  assert.equal(accounts.invites.revoke(revoked.id), true);
  assert.equal((await post('/invite/check', { token: revoked.token })).status, 403);
});

test('tokens are stored hashed', () => {
  const { token } = accounts.invites.create({ uses: 1, days: 1 });
  const row = accounts.database.prepare('SELECT token_hash FROM invite').all() as { token_hash: string }[];
  assert.ok(row.every(({ token_hash }) => token_hash !== token));
});

test('the session cookie is host-only and secure, and a session names the account', async () => {
  const { token } = accounts.invites.create({ uses: 1, days: 1 });
  const reply = await signUp('session@example.test', await inviteCookie(token));
  const cookie = reply.cookies.find((value) => value.startsWith(`${SESSION_COOKIE}=`));
  assert.ok(cookie, reply.cookies.join(' | '));
  assert.match(cookie, /; Secure/i);
  assert.match(cookie, /; HttpOnly/i);
  assert.match(cookie, /; Path=\//i);
  assert.doesNotMatch(cookie, /; Domain=/i);
  const current = await session(pairs(reply.cookies));
  assert.equal(current.user.email, 'session@example.test');
});

test('sign-in works, a wrong password does not, and other origins are refused', async () => {
  const { token } = accounts.invites.create({ uses: 1, days: 1 });
  await signUp('signin@example.test', await inviteCookie(token));
  const good = await post('/sign-in/email', { email: 'signin@example.test', password: 'correct horse battery' });
  assert.equal(good.status, 200);
  const bad = await post('/sign-in/email', { email: 'signin@example.test', password: 'wrong wrong wrong wrong' });
  assert.equal(bad.status, 401);
  const tunnel = await post(
    '/sign-in/email',
    { email: 'signin@example.test', password: 'correct horse battery' },
    { origin: 'https://0123456789abcdef.app.example.test' },
  );
  assert.equal(tunnel.status, 403, 'a sibling tunnel host is not a trusted origin');
});

test('passkeys are Better Auth endpoints, and adding one needs a session', async () => {
  const anonymous = await fetch(`${base}/api/auth/passkey/generate-register-options`);
  assert.equal(anonymous.status, 401);
  const authenticate = await fetch(`${base}/api/auth/passkey/generate-authenticate-options`);
  assert.equal(authenticate.status, 200);
  const options = (await authenticate.json()) as { rpId: string; challenge: string };
  assert.equal(options.rpId, 'app.example.test');
  assert.ok(options.challenge.length > 16);
});

test('sign-in attempts are limited per address', async () => {
  const statuses: number[] = [];
  for (let attempt = 0; attempt < 12; attempt += 1) {
    statuses.push((await post('/sign-in/email', { email: 'limit@example.test', password: 'wrong wrong wrong wrong' })).status);
  }
  assert.ok(statuses.includes(429), statuses.join(','));
});

test('an account cannot be deleted while its Macs would stay admitted', async () => {
  const { token } = accounts.invites.create({ uses: 1, days: 1 });
  const invited = await inviteCookie(token);
  const created = await signUp('leaver@example.test', invited);
  assert.equal(created.status, 200, JSON.stringify(created.body));
  const cookie = pairs(created.cookies);
  const reply = await post('/delete-user', {}, { cookie });
  assert.notEqual(reply.status, 200, JSON.stringify(reply.body));
  assert.equal((await session(cookie))?.user?.email, 'leaver@example.test');
});

test('the relay stops at its account limit', async () => {
  const { token } = accounts.invites.create({ uses: 10, days: 1 });
  const cookie = await inviteCookie(token);
  const statuses: number[] = [];
  for (let index = 0; index < 5; index += 1) {
    client += 1;
    statuses.push((await signUp(`full${index}@example.test`, cookie)).status);
  }
  assert.equal(statuses.at(-1), 403, statuses.join(','));
  const count = accounts.database.prepare('SELECT count(*) AS n FROM user').get() as { n: number };
  assert.equal(count.n, 5);
});

test('the bundle runs the CLI on its own', { skip: !process.env.AB_TEST_BUNDLE }, () => {
  const env = {
    ...process.env,
    AB_PUBLIC_URL: origin,
    AB_ACCOUNTS_DB: join(directory, 'cli.sqlite'),
    AB_ACCOUNTS_SECRET_FILE: process.env.AB_TEST_SECRET_FILE ?? '',
  };
  const out = execFileSync('node', [process.env.AB_TEST_BUNDLE ?? '', 'invite', 'create', '--uses', '2'], { env }).toString();
  assert.match(out, /^https:\/\/app\.example\.test\/invite#/);
});

function loadAccountPage(url: string) {
  const location = new URL(url);
  const context = vm.createContext({
    URL,
    URLSearchParams,
    location,
    document: { getElementById: () => null },
    history: { replaceState() {} },
    fetch: () => Promise.reject(new Error('no network in this test')),
  });
  vm.runInContext(readFileSync(join(import.meta.dirname, '../../../rust/crates/ab-relay-control/assets/account.js'), 'utf8'), context);
  return (next: string | null) => {
    location.search = next === null ? '' : `?next=${encodeURIComponent(next)}`;
    return vm.runInContext('nextPage()', context) as string;
  };
}

test('the page after sign-in stays on this site, however the address is written', () => {
  const nextPage = loadAccountPage('https://app.example.test/sign-in');
  for (const offsite of ['/\\\\review.invalid', '/\\review.invalid/x', '//review.invalid', 'https://review.invalid/', '/\t/review.invalid', '\\\\review.invalid', 'javascript:alert(1)']) {
    const target = new URL(nextPage(offsite), 'https://app.example.test');
    assert.equal(target.origin, 'https://app.example.test', offsite);
  }
  assert.equal(nextPage(null), '/');
  assert.equal(nextPage('/device?code=BCDF-GHJK'), '/device?code=BCDF-GHJK');
  assert.equal(nextPage('/'), '/');
});
