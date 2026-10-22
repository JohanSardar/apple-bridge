export type Recipient = { value: string; kind: 'phone' | 'email'; label: string; contact_id?: string; display_name: string };
export type RecipientResult = { contact_id: string; display_name: string; handles: {value:string;kind:'phone'|'email';label:string}[] };
export type RecipientLookup = { capability: 'available' | 'unavailable'; results: RecipientResult[] };
export function recipientKey(value: string) {
    const trimmed = value.trim().toLowerCase();
    if (trimmed.includes('@')) return trimmed;
    const digits = trimmed.replace(/\D/g, '');
    return digits.length === 10 ? '1' + digits : digits;
}
export function rawRecipient(value: string): Recipient | undefined {
    const trimmed = value.trim();
    if (trimmed.length > 254) return;
    if (/^[^\s@,;]+@[^\s@,;]+\.[^\s@,;]+$/.test(trimmed)) return {value:trimmed,kind:'email',label:'',display_name:trimmed};
    if (/^\+?[\d\s().-]+$/.test(trimmed) && recipientKey(trimmed).length >= 7) return {value:trimmed,kind:'phone',label:'',display_name:trimmed};
}
export function recipientOptions(results: readonly RecipientResult[], query: string): Recipient[] {
    const key = query.trim().toLowerCase(), phone = /^[+\d\s().-]+$/.test(key) ? recipientKey(key) : '';
    return results.flatMap(result => {
        const matching = result.handles.filter(handle => handle.kind === 'email' ? handle.value.toLowerCase().includes(key) : phone && recipientKey(handle.value).includes(phone));
        const choices = result.display_name.toLowerCase().includes(key) || !matching.length ? result.handles : matching;
        return choices.map(handle => ({...handle,contact_id:result.contact_id,display_name:result.display_name}));
    });
}
export async function lookupRecipients(q: string, signal: AbortSignal): Promise<RecipientLookup> {
    const response = await fetch('/web/v1/lookup', {method:'POST',credentials:'same-origin',cache:'no-store',signal,headers:{'Content-Type':'application/json'},body:JSON.stringify({q,limit:8})});
    if (!response.ok) throw new Error('Recipient lookup unavailable');
    return response.json();
}
