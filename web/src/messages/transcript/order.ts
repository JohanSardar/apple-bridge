import type { ThreadItem } from '../../data/types';
import { text } from '../store/types.ts';
import { commandPartIndex } from '../store/message.ts';

export function orderTranscriptItems(items: readonly ThreadItem[]): ThreadItem[] {
    return items.map(item => ({
        item,
        at: Date.parse(text(item.data.command_sent_at) || item.data.sent_at),
        key: text(item.data.command_id) || item.id,
        part: commandPartIndex(item) ?? -1,
    })).sort((a, b) => a.at - b.at || a.key.localeCompare(b.key) || a.part - b.part || a.item.id.localeCompare(b.item.id)).map(value => value.item);
}
