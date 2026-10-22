import { createEffect, createSignal, onCleanup, type Accessor } from 'solid-js';
import { auth } from '../auth/session';
import type { BridgeRecord, Cursor, ViewData } from '../data/types';
export type Item = { contact: BridgeRecord; display_name: string; sort_key: string; initials: string; avatar: { digest: string; variant: string } | null; is_me: boolean | null; conversation_ids: string[] };
export type Section = { key: string; contacts: Item[] };
export type ContactsPage = ViewData & { sections?: Section[]; groups?: { group: BridgeRecord; count: number }[]; containers?: BridgeRecord[]; count?: number; _cursor?: Cursor };
export const text = (value: unknown) => typeof value === 'string' ? value : '';
export const object = (value: unknown): Record<string, unknown> => value && typeof value === 'object' ? value as Record<string, unknown> : {};
export const labeled = (value: unknown) => Array.isArray(value) ? value.map(object) : [];
export function useContacts(args: Accessor<Record<string, unknown>>) {
    const [page, setPage] = createSignal<ContactsPage>({});
    createEffect(() => {
        const live = auth.live(), query = args(); setPage({});
        if (!live) return;
        const subscription = live.subscribe('contacts.list', query, { resetOnReconnect: true });
        createEffect(() => setPage({ ...subscription.data(), _cursor: subscription.cursor() }));
        onCleanup(subscription.dispose);
    });
    return page;
}
export function useContactPreferences() {
    let saved: Record<string, unknown> = {};
    try { saved = object(JSON.parse(localStorage.getItem('ab.contacts.preferences') ?? '{}')); } catch { saved = {}; }
    const [sort, setSort] = createSignal(saved.sort === 'given' ? 'given' : 'family');
    const [display, setDisplay] = createSignal(saved.display === 'family' ? 'family' : 'given');
    const save = () => { if (auth.session()?.kind === 'trusted') try { localStorage.setItem('ab.contacts.preferences', JSON.stringify({ sort: sort(), display: display() })); } catch { return; } };
    return { sort, display, sortBy(value: string) { setSort(value === 'given' ? 'given' : 'family'); save(); }, displayAs(value: string) { setDisplay(value === 'family' ? 'family' : 'given'); save(); } };
}
export function label(value: unknown, fallback: string) { const name = text(value).replace(/^_\$!<(.+)>!\$_$/u, '$1'); return name || fallback; }
export function dateText(value: unknown) {
    const parts=object(value), month=Number(parts.month), day=Number(parts.day), year=Number(parts.year);
    if(!Number.isInteger(month)||!Number.isInteger(day)||month<1||month>12||day<1||day>31)return '';
    if(parts.calendar && parts.calendar!=='gregorian')return [parts.year,month,day].filter(part=>part!==undefined).join('/')+' ('+text(parts.calendar)+')';
    const date=new Date(2000,month-1,day,12);if(Number.isInteger(year))date.setFullYear(year);
    return new Intl.DateTimeFormat(undefined,{month:'long',day:'numeric',...(Number.isInteger(year)?{year:'numeric' as const}:{})}).format(date);
}
export function safeURL(value: unknown) { try { const url = new URL(text(value)); return ['https:', 'http:'].includes(url.protocol) ? url.href : undefined; } catch { return; } }
export function addressText(value: unknown) { const address = object(value); return ['street', 'sub_locality', 'city', 'sub_administrative_area', 'state', 'postal_code', 'country'].map(key => text(address[key])).filter(Boolean).join(', '); }
export function searchable(item: Item) {
    const data = item.contact.data;
    return [item.display_name, data.organization, ...['phones','emails','postal_addresses'].flatMap(key => labeled(data[key]).map(field => key === 'postal_addresses' ? addressText(field.value) : text(field.value)))].join(' ').toLocaleLowerCase();
}
