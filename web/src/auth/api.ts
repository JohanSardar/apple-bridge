export class APIError extends Error {
    constructor(readonly status: number, readonly retryAfter?: number) { super('Request failed'); }
}
export async function api<T>(path: string, body?: unknown, method = body === undefined ? 'GET' : 'POST'): Promise<T> {
    const response = await fetch('/web/v1/' + path, { method, credentials: 'same-origin', signal: AbortSignal.timeout(10000), headers: body === undefined ? undefined : { 'Content-Type': 'application/json' }, body: body === undefined ? undefined : JSON.stringify(body), cache: 'no-store' });
    if (!response.ok)
        throw new APIError(response.status, response.headers.has('Retry-After') ? Number(response.headers.get('Retry-After')) : undefined);
    if (response.status === 204)
        return undefined as T;
    return response.json() as Promise<T>;
}
function decode(value: string) { const binary = atob(value.replace(/-/g, '+').replace(/_/g, '/')); return Uint8Array.from(binary, c => c.charCodeAt(0)).buffer; }
function encode(value: ArrayBuffer) { let binary = ''; for (const byte of new Uint8Array(value))
    binary += String.fromCharCode(byte); return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, ''); }
function serialize(credential: PublicKeyCredential) {
    const response = credential.response;
    const common = { id: credential.id, rawId: encode(credential.rawId), type: credential.type, authenticatorAttachment: credential.authenticatorAttachment, clientExtensionResults: credential.getClientExtensionResults() };
    if (response instanceof AuthenticatorAttestationResponse)
        return { ...common, response: { clientDataJSON: encode(response.clientDataJSON), attestationObject: encode(response.attestationObject), transports: response.getTransports() } };
    const assertion = response as AuthenticatorAssertionResponse;
    return { ...common, response: { clientDataJSON: encode(assertion.clientDataJSON), authenticatorData: encode(assertion.authenticatorData), signature: encode(assertion.signature), userHandle: assertion.userHandle ? encode(assertion.userHandle) : null } };
}
export async function passkey(flow: 'enroll' | 'login' | 'passkeys', kind: 'trusted' | 'public' = 'trusted', token?: string) {
    const options = await api<{
        ceremony_id: string;
        publicKey: Record<string, unknown>;
    }>(flow + '/options', { kind, token });
    const key = options.publicKey;
    key.challenge = decode(key.challenge as string);
    const credentials = (key.excludeCredentials ?? key.allowCredentials) as {
        id: string | ArrayBuffer;
    }[] | undefined;
    for (const credential of credentials ?? [])
        credential.id = decode(credential.id as string);
    if (flow !== 'login') {
        const user = key.user as {
            id: string | ArrayBuffer;
        };
        user.id = decode(user.id as string);
    }
    const credential = await (flow === 'login' ? navigator.credentials.get({ publicKey: key as unknown as PublicKeyCredentialRequestOptions }) : navigator.credentials.create({ publicKey: key as unknown as PublicKeyCredentialCreationOptions }));
    if (!(credential instanceof PublicKeyCredential))
        throw new Error('Passkey cancelled');
    return api<unknown>(flow + '/finish', { kind, token, ceremony_id: options.ceremony_id, credential: serialize(credential) });
}
