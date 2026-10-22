import { createEffect, createMemo, createSignal, onCleanup, type Accessor } from 'solid-js';
import { auth } from '../auth/session';
import type { BridgeRecord, ViewData, ViewName } from '../data/types';
export type Section = { kind: 'store' | 'list' | 'date' | 'daypart' | 'none'; id: string; title: string; list_id?: string };
export type List = { list: BridgeRecord; count?: number; status?: string; group_id: string | null; pinned: boolean | null; sections: Section[] };
export type Lists = ViewData & { smart?: { id: string; count?: number; status?: string }[]; lists?: List[]; tags?: { name: string; count: number }[] };
export type Item = { reminder: BridgeRecord; subtasks: Item[]; attachments: BridgeRecord[] };
export type Items = ViewData & { sections?: { section: Section; items: Item[] }[]; count?: number };
export type Row = { item: Item | null; depth: number; section?: Section };
export { smartNames } from './strings';
export const viewerZone = Intl.DateTimeFormat().resolvedOptions().timeZone;
export const text = (value: unknown) => typeof value === 'string' ? value : '';
export const object = (value: unknown): Record<string, unknown> => value && typeof value === 'object' ? value as Record<string, unknown> : {};
export function useView<T extends ViewData>(name: ViewName, args: Accessor<Record<string, unknown> | undefined>) {
    const [value, setValue] = createSignal<T>({} as T);
    createEffect(() => {
        const live = auth.live(), query = args(); setValue(() => ({} as T));
        if (!live || !query) return;
        const sub = live.subscribe(name, query, { resetOnReconnect: true });
        createEffect(() => { const data = sub.data() as T; setValue(() => data); });
        onCleanup(sub.dispose);
    });
    return value;
}
export function useRows(items: Accessor<Items>, search: Accessor<string>, collapsed: Accessor<Set<string>>) {
    return createMemo(() => {
        const closed = collapsed(), rows: Row[] = [], needle = search().trim().toLocaleLowerCase();
        for (const group of items().sections ?? []) {
            let first = true;
            const visit = (children: Item[], depth: number) => {
                for (const item of children) {
                    const data = item.reminder.data;
                    if (!needle || [data.title, data.notes, data.url, ...(Array.isArray(data.tags) ? data.tags : [])].some(value => text(value).toLocaleLowerCase().includes(needle))) {
                        rows.push({ item, depth: needle ? 0 : depth, section: first && group.section.kind !== 'none' ? group.section : undefined }); first = false;
                    }
                    if (needle || !closed.has(item.reminder.id)) visit(item.subtasks ?? [], depth + 1);
                }
            };
            visit(group.items, 0);
            if (first && !needle && group.section.kind !== 'none') rows.push({ item: null, depth: 0, section: group.section });
        }
        return rows;
    });
}
export function dueLabel(value: unknown) {
    const due = text(value); if (!due) return '';
    const date = new Date(/^\d{4}-\d{2}-\d{2}$/u.test(due) ? due + 'T00:00:00' : due);
    if (!Number.isFinite(date.getTime())) return '';
    return new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', ...(!/^\d{4}-\d{2}-\d{2}$/u.test(due) ? { timeStyle: 'short' as const } : {}) }).format(date);
}
export function overdue(data: Record<string, unknown>) {
    const due = text(data.due); if (!due || data.completed) return false;
    if (/^\d{4}-\d{2}-\d{2}$/u.test(due)) {
        const now = new Date(), today = [now.getFullYear(), String(now.getMonth() + 1).padStart(2, '0'), String(now.getDate()).padStart(2, '0')].join('-');
        return due < today;
    }
    return Date.parse(due) < Date.now();
}

export function sectionLabel(section: Section) {
    if (section.kind !== 'date' || !/^\d{4}-\d{2}-\d{2}$/u.test(section.title)) return section.title;
    return new Intl.DateTimeFormat(undefined, { weekday: 'long', month: 'long', day: 'numeric' }).format(new Date(section.title + 'T00:00:00'));
}
