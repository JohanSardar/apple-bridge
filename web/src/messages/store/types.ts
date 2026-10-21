import type { BridgeRecord, ThreadItem } from '../../data/types';
export type Handle = { id: string; display_name: string; initials: string; address?: string; contact_id?: string; avatar?: { digest: string; variant: string } };
export type Handles = ReadonlyMap<string, Handle>;
export type Part = { index: number; kind: string; text?: string; attachment_id?: string; handle_id?: string; mention?: string; description?: string };
export function fields(value: unknown): Record<string, unknown> { return value && typeof value === 'object' ? value as Record<string, unknown> : {}; }
export function array(value: unknown): unknown[] { return Array.isArray(value) ? value : []; }
export function text(value: unknown): string { return typeof value === 'string' ? value : ''; }
export function messageParts(item: ThreadItem): Part[] {
    const parts = array(item.data.parts).map((value, index) => { const part = fields(value); return { ...part, index: Number(part.index ?? index), kind: text(part.kind) || 'unknown' } as Part; });
    if (!parts.length && text(item.data.text)) parts.push({index:0,kind:'text',text:text(item.data.text)});
    for (const attachment of item.attachments ?? []) if (!parts.some(part => part.attachment_id === attachment.id)) parts.push({index:parts.length,kind:'attachment',attachment_id:attachment.id});
    if (!parts.length) parts.push({index:0,kind:item.data.balloon?'balloon':'unknown'});
    return parts;
}
export function sender(item: ThreadItem | BridgeRecord): string { return text(item.data.sender_handle_id); }
