export type ParsedReminder = { title: string; date: string; time: string };
const days = ['sunday', 'monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday'];
export function localDay(date: Date) { return [date.getFullYear(), String(date.getMonth() + 1).padStart(2, '0'), String(date.getDate()).padStart(2, '0')].join('-'); }
export function parseReminder(value: string, now = new Date()): ParsedReminder {
    const title = value.trim(), empty = { title, date: '', time: '' };
    const match = /^(.*?)\s+(today|tomorrow|in \d{1,3} (?:days?|weeks?)|(?:next )?(?:sunday|monday|tuesday|wednesday|thursday|friday|saturday)|\d{4}-\d{2}-\d{2})(?:\s+(?:at\s+)?(\d{1,2})(?::(\d{2}))?\s*(am|pm)?)?$/iu.exec(title);
    if (!match || !match[1].trim()) return empty;
    const date = new Date(now.getFullYear(), now.getMonth(), now.getDate()), phrase = match[2].toLowerCase();
    if (phrase === 'tomorrow') date.setDate(date.getDate() + 1);
    else if (phrase.startsWith('in ')) { const amount = Number(phrase.split(' ')[1]); date.setDate(date.getDate() + amount * (phrase.includes('week') ? 7 : 1)); }
    else if (phrase !== 'today' && !/^\d{4}/u.test(phrase)) { const day = days.indexOf(phrase.replace('next ', '')); date.setDate(date.getDate() + ((day - date.getDay() + 7) % 7 || 7)); }
    else if (/^\d{4}/u.test(phrase)) { const [year, month, day] = phrase.split('-').map(Number); date.setFullYear(year, month - 1, day); if (localDay(date) !== phrase) return empty; }
    let time = '';
    if (match[3]) {
        let hour = Number(match[3]); const minute = Number(match[4] ?? 0), period = match[5]?.toLowerCase();
        if (minute > 59 || period && (hour < 1 || hour > 12) || !period && hour > 23) return empty;
        if (period) hour = hour % 12 + (period === 'pm' ? 12 : 0);
        time = `${String(hour).padStart(2, '0')}:${String(minute).padStart(2, '0')}`;
    }
    return { title: match[1].trim(), date: localDay(date), time };
}
export function dueFields(date: string, time: string) {
    if (!date) return { due: '', due_all_day: false };
    if (!time) return { due: date, due_all_day: true };
    const instant = new Date(date + 'T' + time);
    if (!Number.isFinite(instant.getTime()) || localDay(instant) !== date || `${String(instant.getHours()).padStart(2, '0')}:${String(instant.getMinutes()).padStart(2, '0')}` !== time) throw new Error('Choose a valid local date and time.');
    return { due: instant.toISOString(), due_all_day: false };
}
