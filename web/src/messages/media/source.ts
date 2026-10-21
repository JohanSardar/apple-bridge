import type { BridgeRecord } from '../../data/types';
import { fields, text } from '../store/types';
export type MediaVariant = 'thumb' | 'display' | 'video' | 'poster' | 'audio' | 'live' | 'original';
export function mediaSource(attachment: BridgeRecord, variant: MediaVariant) {
    const blob = fields(attachment.data.blob), derivative = fields(fields(attachment.data.derivatives)[variant]);
    const digest = text(blob.digest) || text(derivative.digest);
    if (!/^[a-f0-9]{64}$/u.test(digest)) return;
    return `/web/v1/blob/${digest}?record=${encodeURIComponent(attachment.id)}&variant=${variant}`;
}
