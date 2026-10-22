import { object, text, dueLabel } from './store';
const units = new Map([['daily', 'day'], ['weekly', 'week'], ['monthly', 'month'], ['yearly', 'year']]);
const weekdayFormat = new Intl.DateTimeFormat(undefined, { weekday: 'long', timeZone: 'UTC' }), monthFormat = new Intl.DateTimeFormat(undefined, { month: 'long', timeZone: 'UTC' });
const integer = (value: unknown, min: number, max = Number.MAX_SAFE_INTEGER): value is number => typeof value === 'number' && Number.isSafeInteger(value) && value >= min && value <= max;
const finite = (value: unknown, min: number, max = Number.MAX_VALUE): value is number => typeof value === 'number' && Number.isFinite(value) && value >= min && value <= max;
const instant = (value: unknown) => typeof value === 'string' && /^\d{4}-\d{2}-\d{2}T/u.test(value) && Number.isFinite(Date.parse(value));
const weekday = (value: number) => weekdayFormat.format(new Date(Date.UTC(2024, 0, value + 6)));
const numbers = (value: unknown, min: number, max: number): number[] | undefined => value === undefined ? [] : Array.isArray(value) && value.length <= 366 && value.every(item => integer(item, min, max) && item !== 0) ? value : undefined;
const ordinal = (value: number) => value === -1 ? 'Last' : value === 1 ? 'First' : value === 2 ? 'Second' : value === 3 ? 'Third' : value === 4 ? 'Fourth' : value === 5 ? 'Fifth' : String(value);
function rule(value: unknown): string[] | undefined {
    const data = object(value), unit = units.get(text(data.frequency));
    if (!unit || !integer(data.interval, 1)) return;
    const result = [data.interval === 1 ? `Every ${unit}` : `Every ${data.interval} ${unit}s`];
    const days = data.days_of_week === undefined ? [] : data.days_of_week;
    if (!Array.isArray(days) || days.length > 366) return;
    const names: string[] = [];
    for (const value of days) { const day = object(value); if (!integer(day.day, 1, 7) || !integer(day.week, -53, 53)) return; names.push((day.week ? ordinal(day.week) + ' ' : '') + weekday(day.day)); }
    if (names.length) result.push(names.join(', '));
    for (const [field, label, min, max] of [['days_of_month', 'Days of month', -31, 31], ['days_of_year', 'Days of year', -366, 366], ['weeks_of_year', 'Weeks of year', -53, 53], ['positions', 'Matching occurrences', -366, 366]] as const) {
        const values = numbers(data[field], min, max); if (!values) return; if (values.length) result.push(label + ': ' + values.join(', '));
    }
    const months = numbers(data.months, 1, 12); if (!months) return;
    if (months.length) result.push(months.map(month => monthFormat.format(new Date(Date.UTC(2024, month - 1, 1)))).join(', '));
    if (data.first_day_of_week !== undefined && !integer(data.first_day_of_week, 0, 7)) return;
    if (data.first_day_of_week) result.push('Week starts ' + weekday(Number(data.first_day_of_week)));
    if (data.count !== undefined && data.count !== null && !integer(data.count, 0)) return;
    if (data.end !== undefined && data.end !== null && !instant(data.end)) return;
    if (data.end && data.count) return;
    if (data.end) result.push('Until ' + dueLabel(data.end));
    if (data.count) result.push(`After ${data.count} occurrence${data.count === 1 ? '' : 's'}`);
    return result;
}
export function repeatLines(value: unknown): string[] {
    if (!Array.isArray(value) || value.length > 64) return [];
    const rules = value.map(rule); return rules.some(rule => !rule) ? [] : rules.flatMap(rule => rule!);
}
function offsetLabel(seconds: number) {
    if (seconds === 0) return 'At due time';
    const absolute = Math.abs(seconds), [size, unit] = absolute % 86400 === 0 ? [86400, 'day'] : absolute % 3600 === 0 ? [3600, 'hour'] : absolute % 60 === 0 ? [60, 'minute'] : [1, 'second'];
    const count = absolute / Number(size); return `${count} ${unit}${count === 1 ? '' : 's'} ${seconds < 0 ? 'before' : 'after'}`;
}
export function alarmLines(value: unknown): { alerts: string[]; locations: string[] } {
    const empty = { alerts: [], locations: [] };
    if (!Array.isArray(value) || value.length > 64) return empty;
    const alerts: string[] = [], locations: string[] = [];
    for (const entry of value) {
        const data = object(entry);
        if (!integer(data.proximity, 0, 2) || !finite(data.relative_offset, -Number.MAX_VALUE)) return empty;
        if (data.location !== undefined) {
            const place = object(data.location);
            if (data.proximity !== 1 && data.proximity !== 2 || !finite(place.radius, 0)) return empty;
            const coordinates = place.latitude !== undefined || place.longitude !== undefined;
            if (coordinates && (!finite(place.latitude, -90, 90) || !finite(place.longitude, -180, 180))) return empty;
            if (!text(place.title) && !coordinates) return empty;
            locations.push(`${data.proximity === 1 ? 'When arriving' : 'When leaving'}: ${text(place.title) || 'Location'}`, place.radius === 0 ? 'Default radius' : `Radius: ${place.radius} m`);
            if (coordinates) locations.push(`${place.latitude}, ${place.longitude}`);
        } else if (data.proximity !== 0) return empty;
        if (data.absolute_date !== undefined && data.absolute_date !== null) { if (!instant(data.absolute_date)) return empty; alerts.push(dueLabel(data.absolute_date)); }
        else if (!data.location) alerts.push(offsetLabel(data.relative_offset));
    }
    return { alerts, locations };
}
