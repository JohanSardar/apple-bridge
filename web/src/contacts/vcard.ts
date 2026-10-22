import type { Item } from './store';
import { labeled, object, text } from './store';
const escape = (value: unknown) => text(value).replace(/\\/gu, '\\\\').replace(/\r\n|\r|\n/gu, '\\n').replace(/;/gu, '\\;').replace(/,/gu, '\\,');
export function vcard(items: Item[]) {
    const lines: string[] = [];
    for (const item of items) {
        const data = item.contact.data;
        lines.push('BEGIN:VCARD', 'VERSION:3.0', 'FN:' + escape(item.display_name), 'N:' + ['family_name','given_name','middle_name','name_prefix','name_suffix'].map(key => escape(data[key])).join(';'));
        for (const [key, property] of [['organization','ORG'],['job_title','TITLE'],['nickname','NICKNAME']] as const) if (text(data[key])) lines.push(property + ':' + escape(data[key]));
        let labelIndex=0;
        for (const [key, property] of [['phones','TEL'],['emails','EMAIL'],['urls','URL']] as const) for (const entry of labeled(data[key])) if (text(entry.value)) { const group='item'+(++labelIndex); lines.push(group+'.'+property+':'+escape(entry.value)); if(text(entry.label))lines.push(group+'.X-ABLabel:'+escape(text(entry.label).replace(/^_\$!<(.+)>!\$_$/u,'$1'))); }
        for (const entry of labeled(data.postal_addresses)) { const address = object(entry.value); lines.push('ADR:' + ['', '', text(address.street), text(address.city), text(address.state), text(address.postal_code), text(address.country)].map(escape).join(';')); }
        const birthday = object(data.birthday);
        if (birthday.month && birthday.day) lines.push('BDAY:' + (birthday.year ? String(birthday.year).padStart(4,'0') + '-' : '--') + String(birthday.month).padStart(2,'0') + '-' + String(birthday.day).padStart(2,'0'));
        lines.push('END:VCARD');
    }
    return lines.flatMap(line => { const folded: string[] = []; let current = ''; for (const rune of line) { if (new TextEncoder().encode(current + rune).length > 75) { folded.push(current); current = ' '; } current += rune; } folded.push(current); return folded; }).join('\r\n') + '\r\n';
}
export function downloadContacts(items: Item[]) {
    const url = URL.createObjectURL(new Blob([vcard(items)], { type: 'text/vcard;charset=utf-8' }));
    const link = document.createElement('a'); link.href = url; link.download = 'Contacts.vcf'; link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
}
