import { civil, dateKey, day, text, zone } from './store';
export function wallTime(value: string, timeZone: string) {
    const date = new Date(value);
    if (!Number.isFinite(date.getTime())) return '';
    const parts = new Intl.DateTimeFormat('sv-SE', { timeZone, year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hourCycle: 'h23' }).formatToParts(date);
    const part = (name: string) => parts.find(value => value.type === name)?.value;
    return `${part('year')}-${part('month')}-${part('day')}T${part('hour')}:${part('minute')}`;
}
export function instant(value: string, timeZone: string) {
    const target = Date.parse(value + 'Z');
    if (!Number.isFinite(target)) throw new Error('Choose a valid date and time.');
    let candidate = target;
    for (let i = 0; i < 4; i++) {
        const displayed = Date.parse(wallTime(new Date(candidate).toISOString(), timeZone) + 'Z');
        if (displayed === target) return new Date(candidate).toISOString();
        candidate += target - displayed;
    }
    throw new Error('This local time does not exist in the selected time zone.');
}
export function quickEvent(input: string, anchor: Date) {
    const match = /^(.*?)\s+(today|tomorrow|sunday|monday|tuesday|wednesday|thursday|friday|saturday)\s+(\d{1,2})(?::(\d{2}))?\s*(am|pm)?$/iu.exec(input.trim());
    if (!match || !match[1].trim()) return;
    const weekday = ['sunday', 'monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday'].indexOf(match[2].toLowerCase());
    const date = day(anchor, weekday >= 0 ? (weekday - anchor.getDay() + 7) % 7 : match[2].toLowerCase() === 'tomorrow' ? 1 : 0);
    let hour = Number(match[3]); const minute = Number(match[4] ?? 0), meridiem = match[5]?.toLowerCase();
    if (minute > 59 || hour > (meridiem ? 12 : 23) || (meridiem && hour < 1)) return;
    if (meridiem) hour = hour % 12 + (meridiem === 'pm' ? 12 : 0);
    try {
        const start = instant(`${dateKey(date)}T${String(hour).padStart(2, '0')}:${String(minute).padStart(2, '0')}`, zone);
        return { title: match[1].trim(), start, end: new Date(Date.parse(start) + 3600000).toISOString(), all_day: false, time_zone: zone };
    } catch { return; }
}
export function draftBounds(start: Date, end: Date, allDay = false) { return { start: allDay ? dateKey(start) : start.toISOString(), end: allDay ? dateKey(end) : end.toISOString(), all_day: allDay, time_zone: allDay ? '' : zone }; }
export function shifted(data: Record<string, unknown>, minutes: number, resizing = false, days = 0) {
    const start = new Date(text(data.start)), end = new Date(text(data.end));
    if (data.all_day) {
        const offset = days + Math.round(minutes / 1440);
        return { start: resizing ? text(data.start) : dateKey(day(civil(text(data.start)), offset)), end: dateKey(day(civil(text(data.end)), offset)) };
    }
    if (!Number.isFinite(start.getTime()) || !Number.isFinite(end.getTime())) return;
    const localShift = (value: Date) => new Date(value.getFullYear(), value.getMonth(), value.getDate() + days, value.getHours(), value.getMinutes() + minutes, value.getSeconds(), value.getMilliseconds());
    const nextStart = resizing ? start : localShift(start), nextEnd = resizing ? localShift(end) : new Date(nextStart.getTime() + end.getTime() - start.getTime());
    if (nextEnd <= nextStart) return;
    return { start: nextStart.toISOString(), end: nextEnd.toISOString() };
}
