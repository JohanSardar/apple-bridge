import { test, expect } from '@playwright/test';
test('AUTH-PUBLIC-ENROLL public RP excludes localhost credentials', async ({ browserName, playwright }) => {
  test.skip(browserName !== 'chromium' || !process.env.AB_PUBLIC_URL);
  const browser = await playwright.chromium.launch({ args: ['--host-resolver-rules=MAP bridge.test 127.0.0.1', '--no-proxy-server'] });
  try {
    const context = await browser.newContext({ ignoreHTTPSErrors: true });
    const page = await context.newPage();
    const cdp = await context.newCDPSession(page);
    await cdp.send('WebAuthn.enable');
    const { authenticatorId } = await cdp.send('WebAuthn.addVirtualAuthenticator', { options: { protocol: 'ctap2', transport: 'internal', hasResidentKey: true, hasUserVerification: true, isUserVerified: true, automaticPresenceSimulation: true } });
    async function enroll(origin: string, token: string) {
      await page.goto(origin);
      return page.evaluate(async token => {
        const post = async (path: string, data: unknown) => fetch('/web/v1/' + path, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(data) });
        const options = await (await post('enroll/options', { token })).json();
        const decode = (s: string) => Uint8Array.from(atob(s.replace(/-/g, '+').replace(/_/g, '/')), c => c.charCodeAt(0));
        const encode = (b: ArrayBuffer) => btoa(String.fromCharCode(...new Uint8Array(b))).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
        const key = options.publicKey;
        key.challenge = decode(key.challenge); key.user.id = decode(key.user.id);
        for (const item of key.excludeCredentials ?? []) item.id = decode(item.id);
        const credential = await navigator.credentials.create({ publicKey: key }) as PublicKeyCredential;
        const response = credential.response as AuthenticatorAttestationResponse;
        const result = await post('enroll/finish', { ceremony_id: options.ceremony_id, credential: { id: credential.id, rawId: encode(credential.rawId), type: credential.type, response: { attestationObject: encode(response.attestationObject), clientDataJSON: encode(response.clientDataJSON), transports: response.getTransports() }, clientExtensionResults: credential.getClientExtensionResults() } });
        return { status: result.status, rp: key.rp.id };
      }, token);
    }
    expect(await enroll(process.env.AB_LOCAL_URL!, process.env.AB_LOCAL_TOKEN!)).toEqual({ status: 200, rp: 'localhost' });
    await page.goto(process.env.AB_PUBLIC_URL!);
    const localCredential = await cdp.send('WebAuthn.getCredentials', { authenticatorId });
    expect(localCredential.credentials).toHaveLength(1);
    const foreign = await page.evaluate(async () => {
      try { await navigator.credentials.get({ publicKey: { challenge: new Uint8Array(32), rpId: 'localhost', timeout: 1000 } }); return 'accepted'; } catch (error) { return (error as DOMException).name; }
    });
    expect(foreign).toBe('SecurityError');
    expect(await enroll(process.env.AB_PUBLIC_URL!, process.env.AB_PUBLIC_TOKEN!)).toEqual({ status: 200, rp: 'bridge.test' });
    const keys = await cdp.send('WebAuthn.getCredentials', { authenticatorId });
    expect(keys.credentials.map((key: { rpId: string }) => key.rpId).sort()).toEqual(['bridge.test', 'localhost']);
    const status = await page.evaluate(async () => (await fetch('/web/v1/session')).status);
    expect(status).toBe(200);
    expect((await context.cookies(process.env.AB_PUBLIC_URL!)).some(cookie => cookie.name === '__Host-ab_session' && cookie.secure)).toBe(true);
    await context.close();
  } finally { await browser.close(); }
});
