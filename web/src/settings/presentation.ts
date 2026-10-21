import { strings as s } from '../app/strings';
export function lastUsed(value: string, now = new Date()) {
    const date = new Date(value);
    const day = new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
    const today = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
    if (day === today) return s.todayAt + ' ' + date.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' });
    const yesterday = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1).getTime();
    if (day === yesterday) return s.yesterday;
    return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric', ...(date.getFullYear() !== now.getFullYear() ? { year: 'numeric' } : {}) });
}
