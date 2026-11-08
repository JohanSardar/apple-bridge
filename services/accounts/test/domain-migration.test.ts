import assert from 'node:assert/strict';
import { randomBytes } from 'node:crypto';
import { spawn } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { open } from '../src/auth.ts';

import { oldOrigin, newOrigin, cookie, b64, request, enroll, authenticate } from './domain-fixture.ts';

test('parallel origins retain registration authority, shared sessions and RP-bound verification', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'ab-domain-'));
  const common = { databasePath: join(dir, 'accounts.sqlite'), secret: b64(randomBytes(48)), maxAccounts: 2 };
  const original = open({ ...common, publicUrl: oldOrigin, allowRegistration: true });
  await original.migrate();
  const second = open({ ...common, publicUrl: newOrigin, registrationOrigin: oldOrigin, allowRegistration: true });
  try {
    await Promise.all([original.migrate(), second.migrate()]);
    const invite = original.invites.create({ uses: 3, days: 1 });
    const checked = await request(original, oldOrigin, '/invite/check', '', { token: invite.token });
    const invited = cookie(checked);
    const signup = { email: 'fixture@example.test', name: '[ab-test] migration', password: 'fixture password long enough' };
    const denied = await request(second, newOrigin, '/sign-up/email', invited, signup);
    assert.equal(denied.status, 403);
    assert.equal(original.invites.list()[0].uses_left, 3);
    const hook = second.auth.options.databaseHooks!.user!.create!.before!;
    await assert.rejects(() => hook(signup as any, { getSignedCookie() { throw Error('invite must not be read'); } } as any), /Create your account/);
    const created = await request(original, oldOrigin, '/sign-up/email', invited, signup);
    assert.equal(created.status, 200, JSON.stringify(await created.clone().json()));
    assert.equal(original.invites.list()[0].uses_left, 2);
    const oldSession = cookie(created);
    for (const value of created.headers.getSetCookie().filter(value => value.startsWith('__Host-ab_account='))) {
      assert.match(value, /Secure/i); assert.match(value, /HttpOnly/i); assert.match(value, /Path=\//i); assert.doesNotMatch(value, /Domain=/i);
    }
    const signedIn = await request(second, newOrigin, '/sign-in/email', '', signup);
    assert.equal(signedIn.status, 200);
    const newSession = cookie(signedIn);
    const sessionCookies = signedIn.headers.getSetCookie().filter(value => value.startsWith('__Host-ab_account='));
    assert.ok(sessionCookies.length > 0);
    for (const value of sessionCookies) {
      assert.match(value, /Secure/i); assert.match(value, /HttpOnly/i); assert.match(value, /Path=\//i); assert.doesNotMatch(value, /Domain=/i);
    }
    assert.notEqual((await request(second, 'https://customer.new.example', '/sign-in/email', '', signup)).status, 200);
    assert.equal((await (await request(second, newOrigin, '/get-session', oldSession)).json()).user.email, signup.email);
    const oldKey = await enroll(original, oldOrigin, oldSession);
    const newKey = await enroll(second, newOrigin, newSession);
    assert.equal((await authenticate(original, oldOrigin, oldKey)).status, 200);
    assert.equal((await authenticate(second, newOrigin, newKey)).status, 200);
    assert.notEqual((await authenticate(second, newOrigin, oldKey, oldOrigin)).status, 200);
    const listed = await (await request(second, newOrigin, '/passkey/list-user-passkeys', newSession)).json();
    assert.equal(listed.length, 2);
    const oldRow = listed.find((item: any) => item.credentialID === b64(oldKey.id));
    const other = await request(original, oldOrigin, '/sign-up/email', invited, { ...signup, email: 'other-fixture@example.test' });
    assert.equal(other.status, 200);
    const refused = await request(second, newOrigin, '/passkey/delete-passkey', cookie(other), { id: oldRow.id });
    assert.notEqual(refused.status, 200);
    assert.equal((await request(second, newOrigin, '/passkey/delete-passkey', newSession, { id: oldRow.id })).status, 200);
    assert.equal((await (await request(original, oldOrigin, '/passkey/list-user-passkeys', oldSession)).json()).length, 1);
  } finally {
    original.database.close(); second.database.close(); rmSync(dir, { recursive: true, force: true });
  }
});


test('OAuth callback cannot create a new account on the migration origin', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'ab-oauth-domain-'));
  const accounts = open({ publicUrl: newOrigin, databasePath: join(dir, 'accounts.sqlite'), secret: b64(randomBytes(48)), maxAccounts: 5, google: { clientId: 'fixture', clientSecret: 'fixture' } });
  try {
    await accounts.migrate();
    const context = await accounts.auth.$context;
    const provider = context.socialProviders.find(provider => provider.id === 'google')!;
    provider.validateAuthorizationCode = async () => ({ accessToken: 'fixture-access' });
    provider.getUserInfo = async () => ({ user: { id: 'fixture-google', email: 'oauth-fixture@example.test', emailVerified: true, name: '[ab-test] OAuth' }, data: { sub: 'fixture-google' } });
    const start = await request(accounts, newOrigin, '/sign-in/social', '', { provider: 'google', callbackURL: newOrigin });
    assert.equal(start.status, 200);
    const authorization = new URL((await start.json()).url);
    const callback = await request(accounts, newOrigin, `/callback/google?code=fixture&state=${encodeURIComponent(authorization.searchParams.get('state')!)}`, cookie(start));
    assert.equal(callback.status, 403);
    assert.match((await callback.json()).message, /New accounts are disabled/);
    assert.equal((accounts.database.prepare('SELECT count(*) AS n FROM user').get() as any).n, 0);
  } finally {
    accounts.database.close(); rmSync(dir, { recursive: true, force: true });
  }
});

test('immediate transactions commit, roll back and preserve SQL errors', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'ab-transaction-domain-'));
  const accounts = open({ publicUrl: newOrigin, databasePath: join(dir, 'accounts.sqlite'), secret: b64(randomBytes(48)), maxAccounts: 1 });
  try {
    await accounts.migrate();
    const db = accounts.database;
    db.exec('CREATE TABLE fixture(value INTEGER)');
    db.prepare('begin').run();
    db.prepare('INSERT INTO fixture VALUES(1)').run();
    db.prepare('rollback').run();
    assert.equal((db.prepare('SELECT count(*) AS n FROM fixture').get() as any).n, 0);
    db.prepare('begin').run();
    db.prepare('INSERT INTO fixture VALUES(2)').run();
    db.prepare('commit').run();
    assert.equal((db.prepare('SELECT value FROM fixture').get() as any).value, 2);
    assert.throws(() => db.prepare('unrelated invalid SQL'), /syntax error/);
    const holder = spawn(process.execPath, ['--input-type=module', '-e', "import {DatabaseSync} from 'node:sqlite'; const db=new DatabaseSync(process.argv[1]); db.exec('BEGIN IMMEDIATE'); console.log('held'); process.stdin.once('data',()=>{db.exec('ROLLBACK');db.close();process.exit(0)});", join(dir, 'accounts.sqlite')], { stdio: ['pipe', 'pipe', 'pipe'] });
    await new Promise<void>((done, reject) => { holder.stdout.once('data', () => done()); holder.once('error', reject); });
    db.exec('PRAGMA busy_timeout=100');
    const started = Date.now();
    try {
      assert.throws(() => db.prepare('begin').run(), /database is locked/);
      assert.ok(Date.now() - started >= 75 && Date.now() - started < 2000);
    } finally {
      const exited = new Promise<void>(done => holder.once('exit', () => done()));
      holder.stdin.end('release');
      await exited;
    }
    db.prepare('begin').run(); db.prepare('rollback').run();

    assert.equal((db.prepare('PRAGMA integrity_check').get() as any).integrity_check, 'ok');
  } finally {
    accounts.database.close(); rmSync(dir, { recursive: true, force: true });
  }
});
