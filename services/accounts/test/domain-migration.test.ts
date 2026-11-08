import assert from 'node:assert/strict';
import { createHash, generateKeyPairSync, randomBytes, sign } from 'node:crypto';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { open } from '../src/auth.ts';

const oldOrigin = 'https://app.old.example';
const newOrigin = 'https://app.new.example';
const cookie = (response: Response) => response.headers.getSetCookie().map(value => value.split(';')[0]).join('; ');
const hash = (data: string | Buffer) => createHash('sha256').update(data).digest();
const b64 = (data: Buffer) => data.toString('base64url');

function cbor(value: any): Buffer {
  const head = (major: number, length: number) => length < 24 ? Buffer.from([major * 32 + length]) : length < 256 ? Buffer.from([major * 32 + 24, length]) : Buffer.from([major * 32 + 25, length >> 8, length & 255]);
  if (typeof value === 'number') return head(value < 0 ? 1 : 0, value < 0 ? -1 - value : value);
  if (Buffer.isBuffer(value)) return Buffer.concat([head(2, value.length), value]);
  if (typeof value === 'string') return Buffer.concat([head(3, Buffer.byteLength(value)), Buffer.from(value)]);
  const entries = value instanceof Map ? [...value] : Object.entries(value);
  return Buffer.concat([head(5, entries.length), ...entries.flatMap(([key, item]) => [cbor(key), cbor(item)])]);
}

async function request(accounts: ReturnType<typeof open>, origin: string, path: string, cookies = '', body?: any) {
  return accounts.auth.handler(new Request(`${origin}/api/auth${path}`, {
    method: body === undefined ? 'GET' : 'POST',
    headers: { origin, cookie: cookies, 'content-type': 'application/json', 'x-forwarded-for': '198.51.100.9' },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  }));
}

async function enroll(accounts: ReturnType<typeof open>, origin: string, session: string) {
  const options = await request(accounts, origin, '/passkey/generate-register-options', session);
  assert.equal(options.status, 200);
  const challenge = (await options.json()).challenge;
  const key = generateKeyPairSync('ec', { namedCurve: 'prime256v1' });
  const jwk = key.publicKey.export({ format: 'jwk' });
  const id = randomBytes(32);
  const cose = cbor(new Map<any, any>([[1, 2], [3, -7], [-1, 1], [-2, Buffer.from(jwk.x!, 'base64url')], [-3, Buffer.from(jwk.y!, 'base64url')]]));
  const authData = Buffer.concat([hash(new URL(origin).hostname), Buffer.from([0x45, 0, 0, 0, 0]), Buffer.alloc(16), Buffer.from([0, id.length]), id, cose]);
  const response = await request(accounts, origin, '/passkey/verify-registration', `${session}; ${cookie(options)}`, { name: origin, response: {
    id: b64(id), rawId: b64(id), type: 'public-key', clientExtensionResults: {}, response: {
      clientDataJSON: b64(Buffer.from(JSON.stringify({ type: 'webauthn.create', challenge, origin }))),
      attestationObject: b64(cbor({ fmt: 'none', attStmt: {}, authData })), transports: ['internal'],
    },
  } });
  assert.equal(response.status, 200, JSON.stringify(await response.clone().json()));
  return { id, key };
}

async function authenticate(accounts: ReturnType<typeof open>, origin: string, credential: Awaited<ReturnType<typeof enroll>>, rpOrigin = origin) {
  const options = await request(accounts, origin, '/passkey/generate-authenticate-options');
  const challenge = (await options.json()).challenge;
  const data = Buffer.from(JSON.stringify({ type: 'webauthn.get', challenge, origin }));
  const authData = Buffer.concat([hash(new URL(rpOrigin).hostname), Buffer.from([5, 0, 0, 0, 1])]);
  return request(accounts, origin, '/passkey/verify-authentication', cookie(options), { response: {
    id: b64(credential.id), rawId: b64(credential.id), type: 'public-key', clientExtensionResults: {}, response: {
      clientDataJSON: b64(data), authenticatorData: b64(authData), signature: b64(sign('sha256', Buffer.concat([authData, hash(data)]), credential.key.privateKey)),
    },
  } });
}

test('parallel origins retain registration authority, shared sessions and RP-bound verification', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'ab-domain-'));
  const common = { databasePath: join(dir, 'accounts.sqlite'), secret: b64(randomBytes(48)), maxAccounts: 1 };
  const original = open({ ...common, publicUrl: oldOrigin, allowRegistration: true });
  await original.migrate();
  const second = open({ ...common, publicUrl: newOrigin, registrationOrigin: oldOrigin });
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
    assert.equal((await (await request(second, newOrigin, '/get-session', oldSession)).json()).user.email, signup.email);
    const oldKey = await enroll(original, oldOrigin, oldSession);
    const newKey = await enroll(second, newOrigin, newSession);
    assert.equal((await authenticate(original, oldOrigin, oldKey)).status, 200);
    assert.equal((await authenticate(second, newOrigin, newKey)).status, 200);
    assert.notEqual((await authenticate(second, newOrigin, oldKey, oldOrigin)).status, 200);
    const listed = await (await request(second, newOrigin, '/passkey/list-user-passkeys', newSession)).json();
    assert.equal(listed.length, 2);
    const oldRow = listed.find((item: any) => item.credentialID === b64(oldKey.id));
    assert.equal((await request(second, newOrigin, '/passkey/delete-passkey', newSession, { id: oldRow.id })).status, 200);
    assert.equal((await (await request(original, oldOrigin, '/passkey/list-user-passkeys', oldSession)).json()).length, 1);
  } finally {
    original.database.close(); second.database.close(); rmSync(dir, { recursive: true, force: true });
  }
});
