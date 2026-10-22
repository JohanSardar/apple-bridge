import type { BridgeRecord } from '../../data/types';
import { fields, text } from '../store/types';
export type MediaVariant = 'thumb' | 'display' | 'video' | 'poster' | 'audio' | 'live' | 'original';
export function attachmentHasAlpha(attachment: BridgeRecord): boolean {
    if (attachment.data.is_sticker || attachment.data.genmoji) return true;
    const hash = text(fields(attachment.data.media).thumbhash);
    try { const bytes = hash.length <= 136 ? atob(hash) : ''; return bytes.length >= 5 && Boolean(bytes.charCodeAt(2) & 128); } catch { return false; }
}
export function mediaSource(attachment: BridgeRecord, variant: MediaVariant) {
    const blob = fields(attachment.data.blob), derivative = fields(fields(attachment.data.derivatives)[variant]);
    const digest = variant === 'original' ? text(blob.digest) : text(derivative.digest) || text(blob.digest);
    if (!/^[a-f0-9]{64}$/u.test(digest)) return;
    return `/web/v1/blob/${digest}?record=${encodeURIComponent(attachment.id)}&variant=${variant}`;
}
