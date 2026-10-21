import { createResource, createSignal, For, Match, Show, Switch } from 'solid-js';
import './settings.css';
import { Agents, Approvals, Activity } from './AgentAccess';
import { api, passkey } from '../auth/api';
import { auth, endSession } from '../auth/session';
import { appearance, type Theme } from '../app/preferences';
import { router } from '../app/router';
import { strings as s } from '../app/strings';
import { Icon, type IconName } from '../design/Icon';
const sections: [
    string,
    string,
    IconName
][] = [['devices', s.devices, 'laptop'], ['passkeys', s.passkeys, 'key'], ['appearance', s.appearance, 'appearance'], ['agents', s.agents, 'spark'], ['approvals', s.approvals, 'shield'], ['activity', s.activity, 'list']];
type Device = {
    device_id: string;
    kind: string;
    expires_at: string;
};
type Passkey = {
    id: string;
    created_at: string;
};
export default function Settings() {
    const section = () => router.route().section ?? 'devices';
    const [devices, { refetch: reloadDevices }] = createResource(() => section() === 'devices', async (enabled) => enabled ? api<{
        devices: Device[];
    }>('devices') : undefined);
    const [keys, { refetch: reloadKeys }] = createResource(() => section() === 'passkeys', async (enabled) => enabled ? api<{
        passkeys: Passkey[];
    }>('passkeys') : undefined);
    const [error, setError] = createSignal(''), [busy, setBusy] = createSignal(false);
    const action = async (fn: () => Promise<unknown>) => { setError(''); setBusy(true); try {
        await fn();
    }
    catch {
        setError(s.unavailable);
    }
    finally {
        setBusy(false);
    } };
    const title = () => sections.find(row => row[0] === section())?.[1] ?? s.settings;
    return <div class="settings-layout"><aside class="settings-nav"><header class="toolbar glass"><button class="icon-button" aria-label={s.back} onClick={() => router.navigate('/messages')}><Icon name="chevron"/></button><h2>{s.settings}</h2></header><nav aria-label={s.settings}><For each={sections}>{([id, label, icon]) => <a classList={{ selected: section() === id }} aria-current={section() === id ? 'page' : undefined} href={'/settings/' + id} onClick={event => { event.preventDefault(); router.navigate('/settings/' + id); }}><span class={'settings-tile ' + id}><Icon name={icon}/></span>{label}</a>}</For></nav><button class="button danger" onClick={() => void action(() => endSession())}>{s.signOut}</button></aside><main class="settings-content" data-scroll="settings"><h1>{title()}</h1><Show when={error() || devices.error || keys.error}><p role="alert" class="error">{error() || s.unavailable}</p></Show><Switch><Match when={section() === 'appearance'}><section class="group"><div class="group-row"><label for="theme">{s.theme}</label><select id="theme" value={appearance.theme()} onChange={event => appearance.setTheme(event.currentTarget.value as Theme)}><option value="system">{s.system}</option><option value="light">{s.light}</option><option value="dark">{s.dark}</option></select></div><div class="group-row"><label for="text-size">{s.textSize}</label><input id="text-size" type="range" min=".85" max="1.5" step=".05" value={appearance.textSize()} onInput={event => appearance.setTextSize(Number(event.currentTarget.value))}/></div></section><p class="settings-hint caption">{auth.session()?.cache==='persistent'?s.appearanceTrusted:s.appearancePublic}</p></Match><Match when={section() === 'devices'}><p class="settings-hint secondary">{s.devicesHint}</p><section class="group" aria-busy={devices.loading}><For each={devices()?.devices ?? []}>{device => <div class="group-row"><Icon name="devices"/><div class="row-label"><h3>{device.device_id === auth.session()?.device_id ? s.currentDevice : s.browser}</h3><p class="caption">{device.kind === 'public' ? s.publicComputer : s.trustedComputer}</p></div><button class="button danger" disabled={busy()} onClick={() => void action(async () => { await api('devices/' + encodeURIComponent(device.device_id), undefined, 'DELETE'); if (device.device_id === auth.session()?.device_id)
        await endSession(false);
    else
        await reloadDevices(); })}>{s.revoke}</button></div>}</For></section></Match><Match when={section() === 'passkeys'}><p class="settings-hint secondary">{s.passkeysHint}</p><section class="group" aria-busy={keys.loading}><For each={keys()?.passkeys ?? []}>{key => <div class="group-row"><Icon name="key"/><div class="row-label"><h3>{s.passkey}</h3><p class="caption">{new Date(key.created_at).toLocaleDateString('en')}</p></div><button class="button danger" disabled={busy()} onClick={() => void action(async () => { await api('passkeys/' + encodeURIComponent(key.id), undefined, 'DELETE'); await reloadKeys(); })}>{s.remove}</button></div>}</For><div class="group-row"><button class="button subtle" disabled={busy()} onClick={() => void action(async () => { await passkey('passkeys'); await reloadKeys(); })}>{s.addPasskey}</button></div></section></Match><Match when={section() === 'agents'}><Agents /></Match><Match when={section() === 'approvals'}><Approvals /></Match><Match when={section() === 'activity'}><Activity /></Match><Match when={true}><div class="empty"><div class="hero-icon"><Icon name={sections.find(row => row[0] === section())?.[2] ?? 'settings'}/></div><h2>{s.comingSoon}</h2><p>{s.settingsPlaceholder}</p></div></Match></Switch></main></div>;
}
