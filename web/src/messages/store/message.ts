import type { BridgeRecord, ChatSummary, ThreadItem } from '../../data/types.ts';
import { array, fields, text, type Part } from './types.ts';
type LastMessage = NonNullable<ChatSummary['last_message']>;
export function chatService(chat: BridgeRecord | undefined): string { return text(chat?.data.service); }
export function messageService(message: BridgeRecord, chatLabel: string): string { return text(message.data.service) || chatLabel || 'iMessage'; }
export function conversationService(chat: BridgeRecord | undefined, latest: BridgeRecord | LastMessage | null | undefined): string {
    const service = latest ? 'data' in latest ? latest.data.service : latest.service : undefined;
    return text(service) || chatService(chat);
}
export function textService(service: string): boolean { return /^(sms|rcs)$/iu.test(service); }
export function sender(message: BridgeRecord): string { return text(message.data.sender_handle_id); }
export function nativeParts(message: BridgeRecord): Part[] {
    const parts = array(message.data.parts);
    if (parts.length) return parts.filter((part): part is Part => { const value = fields(part); return Number.isSafeInteger(value.index) && Number(value.index) >= 0; });
    return typeof message.data.text === 'string' ? [{ index: 0, kind: 'text', text: message.data.text }] : [];
}
export function partText(message: BridgeRecord, index: number): string | undefined {
    const part = nativeParts(message).find(part => part.index === index);
    return part?.kind === 'text' ? String(part.text ?? (index === 0 ? message.data.text ?? '' : '')) : undefined;
}
export function partAttachmentID(message: BridgeRecord, index: number): string | undefined { return nativeParts(message).find(part => part.index === index)?.attachment_id; }
export function messageText(message: BridgeRecord): string { return text(message.data.text) || array(message.data.parts).map(part => text(fields(part).text)).join(''); }
export function messageParts(item: ThreadItem): Part[] {
    const parts = array(item.data.parts).map((value, index) => { const part = fields(value); return { ...part, index: Number(part.index ?? index), kind: text(part.kind) || 'unknown' } as Part; });
    if (!parts.length && text(item.data.text)) parts.push({index:0,kind:'text',text:text(item.data.text)});
    for (const attachment of item.attachments ?? []) if (!attachment.data.hidden&&!attachment.data.is_sticker&&!parts.some(part => part.attachment_id === attachment.id)) parts.push({index:parts.length,kind:'attachment',attachment_id:attachment.id});
    if (!parts.length) parts.push({index:0,kind:item.data.balloon?'balloon':'unknown'});
    return parts;
}
