#!/usr/bin/env node
// Runs two independent clients against a gateway's OAuth server:
//   oauth4webapi: a strict certified client library, which rejects non-conforming responses.
//   @modelcontextprotocol/sdk auth(): the discovery and registration most MCP clients use.
// Usage: node oauth-clients.mjs <base-url> <approve-command>
// The approve command is run with the pending request's id and code as arguments.
import { execFileSync } from 'node:child_process';
import * as oauth from 'oauth4webapi';
import { auth, discoverAuthorizationServerMetadata, discoverOAuthProtectedResourceMetadata } from '@modelcontextprotocol/sdk/client/auth.js';

const [base, approve] = process.argv.slice(2);
if (!base || !approve) throw new Error('usage: oauth-clients.mjs <base-url> <approve-command>');
const allowHttp = base.startsWith('http:') ? { [oauth.allowInsecureRequests]: true } : {};
const callback = 'https://client.example/callback';
const resource = `${base}/mcp`;
let failures = 0;
const check = (name, ok, detail = '') => { console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` (${detail})` : ''}`); if (!ok) failures += 1; };

async function approved(query) {
  const created = await fetch(`${base}/oauth/pending?${query}`, { method: 'POST', headers: { origin: base } });
  if (created.status !== 201) throw new Error(`pending ${created.status}: ${await created.text()}`);
  const pending = await created.json();
  execFileSync(approve, [pending.id, pending.code], { stdio: 'inherit' });
  const polled = await (await fetch(`${base}/oauth/pending/${encodeURIComponent(pending.id)}`)).json();
  return new URL(polled.redirect);
}

// oauth4webapi: discovery, registration, code flow, refresh.
const issuer = new URL(base);
const asResponse = await oauth.discoveryRequest(issuer, { algorithm: 'oauth2', ...allowHttp });
const as = await oauth.processDiscoveryResponse(issuer, asResponse);
check('RFC 8414 metadata accepted by oauth4webapi', as.issuer === base);
const prmResponse = await oauth.resourceDiscoveryRequest(new URL(resource), allowHttp);
const prm = await oauth.processResourceDiscoveryResponse(new URL(resource), prmResponse);
check('RFC 9728 metadata accepted by oauth4webapi', prm.resource === resource);
check('resource names this issuer', prm.authorization_servers?.[0] === as.issuer);
const registration = await oauth.dynamicClientRegistrationRequest(as, { redirect_uris: [callback], client_name: 'oauth4webapi', token_endpoint_auth_method: 'none' }, allowHttp);
const client = await oauth.processDynamicClientRegistrationResponse(registration);
check('RFC 7591 registration accepted', Boolean(client.client_id));
const verifier = oauth.generateRandomCodeVerifier();
const state = oauth.generateRandomState();
const query = new URLSearchParams({
  response_type: 'code', client_id: client.client_id, redirect_uri: callback, state, resource,
  scope: 'calendar:read offline_access', code_challenge: await oauth.calculatePKCECodeChallenge(verifier), code_challenge_method: 'S256',
});
const back = await approved(query);
const params = oauth.validateAuthResponse(as, client, back, state);
check('RFC 9207 iss and state validated', params.get('iss') === as.issuer);
const exchange = await oauth.authorizationCodeGrantRequest(as, client, oauth.None(), params, callback, verifier, { additionalParameters: { resource }, ...allowHttp });
const tokens = await oauth.processAuthorizationCodeResponse(as, client, exchange);
check('token response accepted', tokens.token_type === 'bearer' && Boolean(tokens.refresh_token));
const refreshed = await oauth.refreshTokenGrantRequest(as, client, oauth.None(), tokens.refresh_token, { additionalParameters: { resource }, ...allowHttp });
const next = await oauth.processRefreshTokenResponse(as, client, refreshed);
check('refresh rotates the token', next.refresh_token && next.refresh_token !== tokens.refresh_token);
const mcp = await fetch(resource, {
  method: 'POST',
  headers: { authorization: `Bearer ${next.access_token}`, 'content-type': 'application/json', accept: 'application/json, text/event-stream' },
  body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'compliance', version: '1' } } }),
});
check('MCP accepts the issued token', mcp.status === 200, `status ${mcp.status}`);

// Unauthenticated MCP must start sign-in: 401 plus a resource_metadata pointer.
const anonymous = await fetch(resource, { method: 'POST', headers: { 'content-type': 'application/json', accept: 'application/json, text/event-stream' }, body: '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' });
const challenge = anonymous.headers.get('www-authenticate') ?? '';
check('anonymous MCP gets 401', anonymous.status === 401);
check('401 points at resource metadata', challenge.includes(`resource_metadata="${base}/.well-known/oauth-protected-resource/mcp"`), challenge);

// The MCP SDK's own flow.
const sdkPrm = await discoverOAuthProtectedResourceMetadata(resource);
check('MCP SDK reads protected resource metadata', sdkPrm.resource === resource);
const sdkAs = await discoverAuthorizationServerMetadata(sdkPrm.authorization_servers[0]);
check('MCP SDK reads authorization server metadata', sdkAs.issuer === base);
let clientInfo; let savedVerifier; let savedTokens; let redirected;
const provider = {
  get redirectUrl() { return callback; },
  get clientMetadata() { return { redirect_uris: [callback], client_name: 'mcp-sdk', token_endpoint_auth_method: 'none', grant_types: ['authorization_code', 'refresh_token'], response_types: ['code'] }; },
  clientInformation: () => clientInfo,
  saveClientInformation: (info) => { clientInfo = info; },
  tokens: () => savedTokens,
  saveTokens: (value) => { savedTokens = value; },
  redirectToAuthorization: (url) => { redirected = url; },
  saveCodeVerifier: (value) => { savedVerifier = value; },
  codeVerifier: () => savedVerifier,
};
const started = await auth(provider, { serverUrl: resource, scope: 'calendar:read' });
check('MCP SDK registers and redirects', started === 'REDIRECT' && redirected?.searchParams.get('resource') === resource, redirected?.search.slice(0, 40));
const sdkBack = await approved(redirected.searchParams.toString());
const finished = await auth(provider, { serverUrl: resource, authorizationCode: sdkBack.searchParams.get('code') });
check('MCP SDK exchanges the code', finished === 'AUTHORIZED' && Boolean(savedTokens?.access_token));

console.log(failures ? `${failures} FAILED` : 'ALL PASSED');
process.exit(failures ? 1 : 0);
