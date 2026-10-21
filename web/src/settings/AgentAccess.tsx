import { createResource, createSignal, For, onCleanup, Show } from 'solid-js';
import { api } from '../auth/api';
import { strings as s } from '../app/strings';
import './agents.css';
export type Level = 'read' | 'approve' | 'full';
const sources = [['messages', s.messages], ['reminders', s.reminders], ['calendar', s.calendar], ['contacts', s.contacts], ['notes', s.notes], ['usage', s.usage]];
const levels: [Level, string, string][] = [['read', s.levelRead, s.levelReadHint], ['approve', s.levelApprove, s.levelApproveHint], ['full', s.levelFull, s.levelFullHint]];
export function AccessPicker(props: { level: Level; setLevel: (level: Level) => void; scopes: string[]; setScopes: (scopes: string[]) => void; allowed?: string[] }) {
    return <><fieldset class="group access-levels"><legend>{s.accessLevel}</legend><For each={levels}>{([level, name, hint]) => <label class="group-row choice"><input type="radio" name="agent-level" value={level} checked={props.level === level} onChange={() => props.setLevel(level)} /><span><strong>{name}</strong><span class="secondary">{hint}</span></span></label>}</For></fieldset><fieldset class="group"><legend>{s.sourceScopes}</legend><For each={sources.filter(([id]) => !props.allowed || props.allowed.some(scope => scope.startsWith(id + ':')))}>{([id, name]) => <label class="group-row"><span>{name}</span><input type="checkbox" aria-label={name} checked={props.scopes.includes(id)} onChange={event => props.setScopes(event.currentTarget.checked ? [...props.scopes, id] : props.scopes.filter(scope => scope !== id))} /></label>}</For></fieldset></>;
}
export function selectedGrants(level: Level, sources: string[], allowed?: string[]) { return sources.map(source => source + (level === 'read' || source === 'usage' || allowed && !allowed.includes(source + ':write') ? ':read' : ':write')); }
type Agent = { id: string; name: string; level: Level; scopes: string[]; created_at: string; last_used?: string; revoked_at?: string };
export function Agents() {
    const [agents, { refetch }] = createResource(() => api<{ agents: Agent[] }>('agents'));
    const [creating, setCreating] = createSignal(false), [name, setName] = createSignal(''), [level, setLevel] = createSignal<Level>('approve'), [scopes, setScopes] = createSignal(['messages']);
    const [token, setToken] = createSignal(''), [url, setURL] = createSignal(location.origin), [busy, setBusy] = createSignal(false), [error, setError] = createSignal(''), [copied, setCopied] = createSignal(false);
    const action = async (fn: () => Promise<void>) => { setBusy(true); setError(''); try { await fn(); } catch { setError(s.unavailable); } finally { setBusy(false); } };
    async function create() { await action(async () => { const value = await api<{ token: string; url: string }>('agents', { name: name(), level: level(), scopes: selectedGrants(level(), scopes()) }); setToken(value.token); setURL(value.url); setCreating(false); setName(''); await refetch(); }); }
    const snippets = () => [[s.grok, `from xai_sdk.tools import mcp

apple_bridge = mcp(
    server_url="${url()}/mcp",
    server_label="apple_bridge",
    authorization="Bearer ${token()}"
)`], [s.claudeCode, `claude mcp add --transport http apple-bridge ${url()}/mcp --header "Authorization: Bearer ${token()}"`], [s.codex, `[mcp_servers.apple_bridge]
url = "${url()}/mcp"
http_headers = { Authorization = "Bearer ${token()}" }`], [s.curl, `curl ${url()}/v1/op/messages.chats \
  -H 'Authorization: Bearer ${token()}' \
  -H 'Content-Type: application/json' \
  -d '{}'`]];
    onCleanup(() => setToken(''));
    return <div class="agent-access"><p class="secondary">{s.agentHint}</p><Show when={error() || agents.error}><p class="error" role="alert">{error() || s.unavailable}</p></Show><Show when={!token()} fallback={<section class="token-reveal"><h2>{s.tokenOnce}</h2><div class="group token-value"><label for="agent-token">{s.tokenLabel}</label><textarea id="agent-token" readonly value={token()} /><button class="button" onClick={() => void action(async () => { await navigator.clipboard.writeText(token()); setCopied(true); })}>{copied() ? s.copied : s.copy}</button></div><h2>{s.snippets}</h2><For each={snippets()}>{([title, text]) => <details class="group snippet"><summary>{title}</summary><pre>{text}</pre></details>}</For><Connector url={url()} /><button class="button" onClick={() => { setToken(''); setCopied(false); }}>{s.done}</button></section>}><Show when={creating()} fallback={<button class="button" onClick={() => setCreating(true)}>{s.connectAgent}</button>}><form onSubmit={event => { event.preventDefault(); void create(); }}><div class="group"><div class="group-row"><label for="agent-name">{s.agentName}</label><input id="agent-name" autofocus maxlength="120" required value={name()} onInput={event => setName(event.currentTarget.value)} /></div></div><AccessPicker level={level()} setLevel={setLevel} scopes={scopes()} setScopes={setScopes} /><div class="agent-actions"><button class="button subtle" type="button" onClick={() => setCreating(false)}>{s.cancel}</button><button class="button" disabled={busy() || !name().trim() || !scopes().length}>{s.connectAgent}</button></div></form></Show><Show when={agents()?.agents.length} fallback={<p class="empty-state">{s.noAgents}</p>}><section class="group agent-list"><For each={agents()?.agents}>{agent => <div class="group-row agent-row"><div><h3>{agent.name}</h3><p>{levels.find(([id]) => id === agent.level)?.[1]} · {agent.scopes.join(', ')}</p><p class="caption">{agent.revoked_at ? s.revoked : agent.last_used ? s.lastUsed + ' ' + new Date(agent.last_used).toLocaleString() : s.neverUsed}</p></div><button class="button danger" disabled={busy() || Boolean(agent.revoked_at)} onClick={() => void action(async () => { await api('agents/' + encodeURIComponent(agent.id), undefined, 'DELETE'); await refetch(); })}>{agent.revoked_at ? s.revoked : s.revoke}</button></div>}</For></section></Show><Connector url={location.origin} /></Show></div>;
}
function Connector(props: { url: string }) { return <section class="group connector"><h3>{s.connectorUIs}</h3><p class="secondary">{s.connectorHint}</p><label>{s.connectorURL}<input readonly value={props.url + '/mcp'} /></label></section>; }
