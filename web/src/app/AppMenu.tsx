import { createSignal, For, Show } from 'solid-js';
import { Icon, type IconName } from '../design/Icon';
import { router } from './router';
import { strings as s } from './strings';
const apps: { name: string; icon: IconName; path: string; enabled: boolean }[] = [
    { name: s.messages, icon: 'messages', path: '/messages', enabled: true },
    { name: s.reminders, icon: 'reminders', path: '/reminders', enabled: true },
    { name: s.calendar, icon: 'calendar', path: '/calendar', enabled: true },
    { name: s.contacts, icon: 'contacts', path: '/contacts', enabled: false },
    { name: s.notes, icon: 'notes', path: '/notes', enabled: false }
];
export function AppMenu() {
    const [open, setOpen] = createSignal(false);
    return <div class="app-selector"><button class="icon-button app-menu-control glass" aria-label={s.switchApp} aria-expanded={open()} onClick={() => setOpen(!open())}><Icon name="grid"/></button><Show when={open()}><div class="app-menu glass" role="group" aria-label={s.appMenu} onKeyDown={event => { if (event.key === 'Escape') setOpen(false); }}><For each={apps}>{app => <button disabled={!app.enabled} onClick={() => { setOpen(false); router.navigate(app.path); }}><span class={'app-icon app-icon-' + app.icon}><Icon name={app.icon}/></span><span>{app.name}<Show when={!app.enabled}><small>{s.comingSoon}</small></Show></span><Show when={router.route().path.startsWith(app.path)}><Icon name="check"/></Show></button>}</For><button class="settings-menu-item" onClick={() => router.navigate('/settings/devices')}><Icon name="settings"/><span>{s.settings}</span></button></div></Show></div>;
}
