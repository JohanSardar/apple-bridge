import { fields, text, type Handles } from '../store/types';
export const wording = {
    sending:'Sending…',sent:'Sent',deliveryUnknown:'Not sure if delivered',
    delivered: 'Delivered', deliveredQuietly: 'Delivered Quietly', notDelivered: 'Not Delivered', edited: 'Edited', you: 'You', unsent: (name: string) => `${name} unsent a message`, replies: (count: number) => `${count} ${count === 1 ? 'Reply' : 'Replies'}`, newMessages: (count: number) => `↓ ${count} new`,
    reactions: {love:'❤️',like:'👍',dislike:'👎',laugh:'😂',emphasize:'‼️',question:'❓'} as Record<string,string>
};
export function time(value: string, locale?: string) { return new Intl.DateTimeFormat(locale, {hour:'numeric',minute:'2-digit'}).format(new Date(value)); }
export function dateLabel(value: string, now = new Date(), locale?: string) {
    const date = new Date(value), today = new Date(now.getFullYear(), now.getMonth(), now.getDate()), day = new Date(date.getFullYear(), date.getMonth(), date.getDate());
    const days = Math.round((today.getTime()-day.getTime())/86400000);
    if (days === 0) return 'Today';
    if (days === 1) return 'Yesterday';
    return new Intl.DateTimeFormat(locale, days > 1 && days < 7 ? {weekday:'long'} : {month:'short',day:'numeric',...(date.getFullYear()!==now.getFullYear()?{year:'numeric'}:{weekday:'short'})}).format(date);
}
export type HeaderParts={day:string;time:string;separator:string};
export function headerParts(value:string,now=new Date(),locale?:string):HeaderParts {
    const date=new Date(value),today=new Date(now.getFullYear(),now.getMonth(),now.getDate()),day=new Date(date.getFullYear(),date.getMonth(),date.getDate()),days=Math.round((today.getTime()-day.getTime())/86400000);
    return {day:dateLabel(value,now,locale),time:time(value,locale),separator:days>=0&&days<7?' ':' at '};
}
export function header(value: string, now?: Date, locale?: string) { const parts=headerParts(value,now,locale);return `${parts.day}${parts.separator}${parts.time}`; }
export function name(id: string, handles: Handles) { return handles.get(id)?.display_name || 'Unknown sender'; }
export function initials(value: string) { return value.trim().split(/\s+/u).slice(0,2).map(word=>Array.from(word)[0]??'').join('').toLocaleUpperCase(); }
export function names(values: string[]) { return values.length <= 2 ? values.join(' & ') : `${values[0]}, ${values[1]} & ${values.length-2} ${values.length===3?'other':'others'}`; }
export function bytes(size: number) { return size < 1024 ? `${size} B` : size < 1048576 ? `${Math.round(size/1024)} KB` : `${(size/1048576).toFixed(1)} MB`; }
export function duration(seconds: number) { return `${Math.floor(seconds/60)}:${String(Math.floor(seconds%60)).padStart(2,'0')}`; }
export function groupEvent(value: unknown, actor: string, handles: Handles) {
    const event = fields(value), kind = text(event.kind ?? event.type), target = name(text(event.handle_id ?? event.target_handle_id), handles);
    if (/name|rename/u.test(kind)||!kind&&text(event.title)) return `${actor} named the conversation “${text(event.name ?? event.title)}”`;
    if (/photo/u.test(kind)) return `${actor} ${/remove/u.test(kind)?'removed':'changed'} the group photo`;
    if (/add/u.test(kind)) return `${actor} added ${target}`;
    if (/remove/u.test(kind)) return `${actor} removed ${target}`;
    if (/left|leave/u.test(kind)) return `${actor} left the conversation`;
    return text(event.text) || `${actor} updated the conversation`;
}
