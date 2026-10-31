import { test } from 'node:test';
import assert from 'node:assert/strict';
import type { ThreadItem } from '../../data/types';
import { orderTranscriptItems } from './order.ts';

test('transcript order preserves captured chronology and multipart local command ties without mutating inputs', () => {
    const item = (id: string, sent_at: string, data = {}) => ({ id, data: { sent_at, ...data } }) as ThreadItem;
    const source = [
        item('part-2', '2026-10-31T16:00:00Z', { command_id: 'command', command_sent_at: '2026-10-31T15:00:00Z', command_part_index: 2 }),
        item('early', '2026-10-31T14:00:00Z'),
        item('late', '2026-10-31T16:00:00Z'),
        item('part-1-b', '2026-10-31T16:00:00Z', { command_id: 'command', command_sent_at: '2026-10-31T15:00:00Z', command_part_index: 1 }),
        item('part-1-a', '2026-10-31T16:00:00Z', { command_id: 'command', command_sent_at: '2026-10-31T15:00:00Z', command_part_index: 1 }),
    ];
    const original = [...source];
    assert.deepEqual(orderTranscriptItems(source).map(item => item.id), ['early', 'part-1-a', 'part-1-b', 'part-2', 'late']);
    assert.deepEqual(source, original);
    const changed = source.map(item => item.id === 'late' ? { ...item, data: { ...item.data, sent_at: '2026-10-31T13:00:00Z' } } : item);
    assert.equal(orderTranscriptItems(changed)[0].id, 'late');
});
