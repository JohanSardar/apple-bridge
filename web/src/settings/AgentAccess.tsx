import { createEffect, createResource, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { api } from '../auth/api';
import { auth } from '../auth/session';
import { strings as s } from '../app/strings';
import { Icon } from '../design/Icon';
import { Toggle, Disclosure } from '../design/Controls';
import { lastUsed } from './presentation';
import './agents.css';
export type Level = 'read' | 'approve' | 'full';
const sources = [['messages', s.messages], ['reminders', s.reminders], ['calendar', s.calendar], ['contacts', s.contacts], ['notes', s.notes], ['usage', s.usage]];
const levels: [Level, string, string][] = [['read', s.levelRead, s.levelReadHint], ['approve', s.levelApprove, s.levelApproveHint], ['full', s.levelFull, s.levelFullHint]];
export function AccessPicker(props: { level: Level; setLevel: (level: Level) => void; scopes: string[]; setScopes: (scopes: string[]) => void; allowed?: string[] }) {
    function navigate(event: KeyboardEvent) {
        const index = levels.findIndex(([level]) => level === props.level);
        const next = event.key === 'Home' ? 0 : event.key === 'End' ? levels.length - 1 : ['ArrowDown', 'ArrowRight'].includes(event.key) ? (index + 1) % levels.length : ['ArrowUp', 'ArrowLeft'].includes(event.key) ? (index + levels.length - 1) % levels.length : -1;
        if (next < 0) return;
        event.preventDefault(); props.setLevel(levels[next][0]);
        (event.currentTarget as HTMLElement).querySelector<HTMLButtonElement>('[data-level="' + levels[next][0] + '"]')?.focus();
    }
    return <><section class="access-section"><h3>{s.accessLevel}</h3><div class="group access-levels" role="radiogroup" aria-label={s.accessLevel} onKeyDown={navigate}><For each={levels}>{([level, name, hint]) => <button type="button" class="group-row access-choice" role="radio" data-level={level} aria-label={name} aria-checked={props.level === level} aria-describedby={'level-hint-' + level} tabIndex={props.level === level ? 0 : -1} onClick={() => props.setLevel(level)}><span><strong>{name}</strong><span class="secondary" id={'level-hint-' + level}>{hint}</span></span><Icon name="check" class={props.level === level ? 'level-check selected' : 'level-check'} /></button>}</For></div></section><section class="access-section"><h3>{s.sourceScopes}</h3><div class="group"><For each={sources.filter(([id]) => !props.allowed || props.allowed.some(scope => scope.startsWith(id + ':')))}>{([id, name]) => <div class="group-row source-choice"><span>{name}</span><Toggle label={name} checked={props.scopes.includes(id)} onChange={checked => props.setScopes(checked ? [...props.scopes, id] : props.scopes.filter(scope => scope !== id))} /></div>}</For></div></section></>;
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

bridge = mcp(
    server_url="${url()}/mcp",
    server_label="bridge",
    authorization="Bearer ${token()}"
)`], [s.claudeCode, `claude mcp add --transport http bridge ${url()}/mcp --header "Authorization: Bearer ${token()}"`], [s.codex, `[mcp_servers.bridge]
url = "${url()}/mcp"
http_headers = { Authorization = "Bearer ${token()}" }`], [s.curl, `curl ${url()}/v1/op/messages.chats \
  -H 'Authorization: Bearer ${token()}' \
  -H 'Content-Type: application/json' \
  -d '{}'`]];
    onCleanup(() => setToken(''));
    return <div class="agent-access"><p class="secondary">{s.agentHint}</p><Show when={error() || agents.error}><p class="error" role="alert">{error() || s.unavailable}</p></Show><Show when={!token()} fallback={<section class="token-reveal"><h2>{s.tokenOnce}</h2><div class="group token-value"><label for="agent-token">{s.tokenLabel}</label><textarea id="agent-token" readonly value={token()} /><button class="button" onClick={() => void action(async () => { await navigator.clipboard.writeText(token()); setCopied(true); })}>{copied() ? s.copied : s.copy}</button></div><h2>{s.snippets}</h2><For each={snippets()}>{([title, text]) => <Disclosure class="group snippet" title={title}><pre>{text}</pre></Disclosure>}</For><Connector url={url()} /><button class="button" onClick={() => { setToken(''); setCopied(false); }}>{s.done}</button></section>}><Show when={creating()} fallback={<button class="button" onClick={() => setCreating(true)}>{s.connectAgent}</button>}><form onSubmit={event => { event.preventDefault(); void create(); }}><div class="group"><div class="group-row"><label for="agent-name">{s.agentName}</label><input id="agent-name" autofocus maxlength="120" required value={name()} onInput={event => setName(event.currentTarget.value)} /></div></div><AccessPicker level={level()} setLevel={setLevel} scopes={scopes()} setScopes={setScopes} /><div class="agent-actions"><button class="button subtle" type="button" onClick={() => setCreating(false)}>{s.cancel}</button><button class="button" disabled={busy() || !name().trim() || !scopes().length}>{s.connectAgent}</button></div></form></Show><Show when={agents()?.agents.length} fallback={<p class="empty-state">{s.noAgents}</p>}><section class="group agent-list"><For each={agents()?.agents}>{agent => <div class="group-row agent-row"><div><h3>{agent.name}</h3><p>{levels.find(([id]) => id === agent.level)?.[1]} · {[...new Set(agent.scopes.map(scope => sources.find(([id]) => id === scope.split(':')[0])?.[1] ?? s.otherSources))].join(', ')}</p><p class="caption">{agent.revoked_at ? s.revoked : agent.last_used ? lastUsed(agent.last_used) : s.neverUsed}</p></div><button class="button danger" disabled={busy() || Boolean(agent.revoked_at)} onClick={() => void action(async () => { await api('agents/' + encodeURIComponent(agent.id), undefined, 'DELETE'); await refetch(); })}>{agent.revoked_at ? s.revoked : s.revoke}</button></div>}</For></section></Show><Connector url={location.origin} /></Show></div>;
}
function Connector(props: { url: string }) { return <section class="group connector"><h3>{s.connectorUIs}</h3><p class="secondary">{s.connectorHint}</p><label>{s.connectorURL}<input readonly value={props.url + '/mcp'} /></label></section>; }
type Approval = { command_id: string; principal: string; operation: string; owner_arguments: Record<string, unknown>; expires_at: string };
export function Approvals() {
    const [agents] = createResource(() => api<{ agents: Agent[] }>('agents'));
    const [rows, { refetch }] = createResource(() => api<{ approvals: Approval[] }>('approvals'));
    const [now, setNow] = createSignal(Date.now()), [error, setError] = createSignal(''), [busy, setBusy] = createSignal('');
    createEffect(() => { auth.approvalRevision(); void refetch(); });
    onMount(() => { const timer = setInterval(() => { setNow(Date.now()); }, 1000); onCleanup(() => clearInterval(timer)); });
    const remaining = (at: string) => Math.min(1440, Math.max(0, Math.ceil((Date.parse(at) - now()) / 60000)));
    async function decide(id: string, decision: string) { setBusy(id); setError(''); try { await api('approvals/' + encodeURIComponent(id), { decision }); await refetch(); } catch { setError(s.unavailable); } finally { setBusy(''); } }
    return <div class="agent-access"><p class="secondary">{s.approvalHint}</p><Show when={rows.error || error()}><p class="error" role="alert">{error() || s.unavailable}</p></Show><Show when={rows()?.approvals.length} fallback={<div class="empty-state"><h2>{s.noApprovals}</h2><p class="secondary">{s.noApprovalsHint}</p></div>}><For each={rows()?.approvals}>{row => <article class="group approval-card"><header><h2>{row.operation}</h2><p class="caption">{agents()?.agents.find(agent => agent.id === row.principal)?.name ?? row.principal}</p></header><h3>{s.ownerArguments}</h3><pre>{typeof row.owner_arguments.text === 'string' ? row.owner_arguments.text : JSON.stringify(row.owner_arguments, null, 2)}</pre><details><summary>{s.details}</summary><pre>{JSON.stringify(row.owner_arguments, null, 2)}</pre></details><p class="caption">{s.timeLeft}: {remaining(row.expires_at) ? remaining(row.expires_at) + ' ' + s.minutesLeft : s.expired}</p><div class="agent-actions"><button class="button danger" disabled={Boolean(busy()) || !remaining(row.expires_at)} onClick={() => void decide(row.command_id, 'reject')}>{s.reject}</button><button class="button" disabled={Boolean(busy()) || !remaining(row.expires_at)} onClick={() => void decide(row.command_id, 'approve')}>{s.approve}</button></div></article>}</For></Show></div>;
}
type Activity = { id: string; principal: string; operation: string; kind: string; state: string; at: string; count: number; argument_bytes: number; argument_hash: string };
export function Activity() {
    const [agents] = createResource(() => api<{ agents: Agent[] }>('agents'));
    const [principal, setPrincipal] = createSignal(''), [operation, setOperation] = createSignal(''), [filter, setFilter] = createSignal('');
    const [rows] = createResource(filter, query => api<{ activity: Activity[] }>('activity' + query));
    return <div class="agent-access"><p class="secondary">{s.activityHint}</p><form class="group activity-filter" onSubmit={event => { event.preventDefault(); setFilter('?' + new URLSearchParams({ principal: principal(), operation: operation() })); }}><label>{s.filterAgent}<select value={principal()} onChange={event => setPrincipal(event.currentTarget.value)}><option value="">{s.allAgents}</option><For each={agents()?.agents}>{agent => <option value={agent.id}>{agent.name}</option>}</For></select></label><label>{s.filterOperation}<input value={operation()} onInput={event => setOperation(event.currentTarget.value)} /></label><button class="button" type="submit">{s.applyFilter}</button></form><Show when={rows.error}><p role="alert" class="error">{s.unavailable}</p></Show><Show when={rows()?.activity.length} fallback={<p class="empty-state">{s.noActivity}</p>}><section class="group"><For each={rows()?.activity}>{row => <article class="group-row activity-row"><div><h3>{row.operation}</h3><p>{row.kind === 'read' ? row.count + ' ' + s.reads : row.state.replaceAll('_', ' ')}</p><p class="caption">{agents()?.agents.find(agent => agent.id === row.principal)?.name ?? row.principal}</p><Show when={row.kind === 'command'}><p class="caption">{row.argument_bytes} B · {row.argument_hash.slice(0, 12)}</p></Show></div><time datetime={row.at}>{new Date(row.at).toLocaleString()}</time></article>}</For></section></Show></div>;
}
