import { createEffect, createSignal, onCleanup, type Accessor } from 'solid-js';
import { auth } from '../auth/session';
import type { BridgeRecord, ViewData, ViewName } from '../data/types';
export type CalendarView = 'day' | 'week' | 'month' | 'year' | 'list';
export type CalendarEntry = { event: BridgeRecord; overlap_group: string; column: number; columns: number };
export type CalendarPage = ViewData & { events?: CalendarEntry[]; calendars?: BridgeRecord[]; coverage_notes?: string[] };
export const views: CalendarView[] = ['day', 'week', 'month', 'year', 'list'];
export const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
export const text = (value: unknown) => typeof value === 'string' ? value : '';
export const object = (value: unknown): Record<string, unknown> => value && typeof value === 'object' ? value as Record<string, unknown> : {};
export const dateKey = (date: Date) => `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;
export function civil(value: string) {
    const match = /^(\d{4})-(\d{2})-(\d{2})$/u.exec(value);
    if (!match) return new Date(NaN);
    const date = new Date(Number(match[1]), Number(match[2]) - 1, Number(match[3]));
    return dateKey(date) === value ? date : new Date(NaN);
}
export const day = (date: Date, offset = 0) => new Date(date.getFullYear(), date.getMonth(), date.getDate() + offset);
export const month = (date: Date, offset = 0) => new Date(date.getFullYear(), date.getMonth() + offset, 1);
export const week = (date: Date) => day(date, -date.getDay());
export function range(view: CalendarView, date: Date, searching = false): [Date, Date] {
    if (searching) return [new Date(date.getFullYear() - 2, date.getMonth(), date.getDate()), new Date(date.getFullYear() + 2, date.getMonth(), date.getDate())];
    if (view === 'day') return [day(date), day(date, 1)];
    if (view === 'week') return [week(date), day(week(date), 7)];
    if (view === 'month') return [week(month(date)), day(week(month(date)), 42)];
    if (view === 'year') return [new Date(date.getFullYear(), 0, 1), new Date(date.getFullYear() + 1, 0, 1)];
    return [day(date), day(date, 90)];
}
export function eventDate(entry: CalendarEntry, field: 'start' | 'end') {
    return entry.event.data.all_day ? civil(text(entry.event.data[field])) : new Date(text(entry.event.data[field]));
}
export function eventsOn(events: CalendarEntry[], date: Date) {
    const start = day(date), end = day(date, 1);
    return events.filter(entry => eventDate(entry, 'start') < end && eventDate(entry, 'end') > start);
}
export function useCalendarView<T extends ViewData>(name: ViewName, args: Accessor<Record<string, unknown> | undefined>) {
    const [data, setData] = createSignal<T>({} as T);
    createEffect(() => {
        const live = auth.live(), query = args(); setData(() => ({} as T));
        if (!live || !query) return;
        const subscription = live.subscribe(name, query, { resetOnReconnect: true });
        createEffect(() => setData(() => subscription.data() as T));
        onCleanup(subscription.dispose);
    });
    return data;
}
export function color(value: unknown) {
    const raw = text(value);
    if (/^#[0-9a-f]{6}$/iu.test(raw)) return raw;
    try { const parts: unknown = JSON.parse(raw); if (Array.isArray(parts) && parts.length >= 3 && parts.slice(0, 3).every(value => typeof value === 'number' && value >= 0 && value <= 1)) return `rgb(${parts.slice(0, 3).map(value => Math.round(Number(value) * 255)).join(' ')})`; } catch { return '#2877cf'; }
    return '#2877cf';
}
export function safeURL(value: unknown) {
    try { const url = new URL(text(value)); return ['http:', 'https:'].includes(url.protocol) ? url.href : undefined; } catch { return; }
}
export const dateLabel = (date: Date, options: Intl.DateTimeFormatOptions = { dateStyle: 'long' }) => new Intl.DateTimeFormat(undefined, options).format(date);
export const timeLabel = (date: Date) => dateLabel(date, { hour: 'numeric', minute: '2-digit' });
