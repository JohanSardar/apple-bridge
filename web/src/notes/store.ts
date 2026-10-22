import { createEffect, createMemo, createSignal, onCleanup, type Accessor } from 'solid-js';
import { auth } from '../auth/session';
import type { BridgeRecord, Cursor, ViewData } from '../data/types';
export type Note = { note: BridgeRecord; preview: string; thumbnail: null | { digest: string; variant: string }; pinned: boolean | null; read_only: boolean };
export type Folder = { folder: BridgeRecord; count: number; account_id: string; parent_id: string | null; read_only: boolean; recently_deleted: boolean };
export type NotesPage = ViewData & { _cursor?: Cursor; count?: number; accounts?: { id: string; title: string; count: number; folder_ids: string[] }[]; folders?: Folder[]; sections?: { key: string; title: string; notes: Note[] }[] };
export const text = (value: unknown) => typeof value === 'string' ? value : '';
export const object = (value: unknown): Record<string, unknown> => value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {};
export function useNotes(args: Accessor<Record<string, unknown>>) {
    const [page, setPage] = createSignal<NotesPage>({});
    const queryKey = createMemo(() => JSON.stringify(args()));
    createEffect(() => {
        const live = auth.live(), query = JSON.parse(queryKey()) as Record<string, unknown>;
        setPage({});
        if (!live) return;
        const subscription = live.subscribe('notes.list', query, { cache: false, resetOnReconnect: true });
        createEffect(() => setPage({ ...subscription.data(), _cursor: subscription.cursor() }));
        onCleanup(subscription.dispose);
    });
    return page;
}
export function matches(entry: Note, query: string) {
    const data = entry.note.data, needle = query.trim().toLocaleLowerCase();
    if (!needle) return true;
    const attachments = Array.isArray(data.attachments) ? data.attachments : [];
    return [data.title, ...(!data.locked ? [data.plain_text, entry.preview, ...attachments.map(value => object(value).name)] : [])].some(value => text(value).toLocaleLowerCase().includes(needle));
}
export function noteDate(value: unknown) {
    const date = new Date(text(value));
    return Number.isFinite(date.getTime()) ? new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' }).format(date) : '';
}
export function blobURL(digest: string, variant: string, record: string) {
    if (!/^[a-f0-9]{64}$/u.test(digest)) return;
    return `/web/v1/blob/${digest}?variant=${encodeURIComponent(variant)}&record=${encodeURIComponent(record)}`;
}
export function useNoteRecord(id: Accessor<string | undefined>) {
    const [record, setRecord] = createSignal<BridgeRecord>();
    createEffect(() => {
        const live = auth.live(), selected = id();
        setRecord();
        if (!live || !selected) return;
        const subscription = live.subscribe('record', { collection: 'notes.note', id: selected }, { cache: false, resetOnReconnect: true });
        createEffect(() => setRecord(subscription.data().records?.find(value => value.id === selected && value.collection === 'notes.note')));
        onCleanup(subscription.dispose);
    });
    return record;
}
