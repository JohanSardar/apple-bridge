import { test } from 'node:test';
import assert from 'node:assert/strict';

test('an unchanged thread cursor reuses the snapshot while changes and recovered errors publish', async () => {
    Object.assign(globalThis, {
        window: { addEventListener() {} },
        location: { href: 'https://example.test/' },
        WebSocket: class { static OPEN = 1; readyState = 0; close() {} },
    });
    const { LiveClient } = await import('./live.ts');
    const client = new LiveClient(() => {});
    const view = client.subscribe('thread', { chat_id: 'chat', anchor: 'latest' }, { cache: false });
    const implementation = client as any, sub = [...implementation.subscriptions.values()][0] as any;
    const update = sub.update;
    let publications = 0;
    sub.update = (value: unknown) => { publications++; update(value); };
    const cursor = { epoch: 'first', subscription_revision: 1, sequence: 10 };
    const snap = (next = cursor) => implementation.receive({ t: 'snap', sub: sub.id, cursor: next, data: { items: [], older: 42 } });
    snap();
    const first = view.data();
    snap();
    assert.equal(view.data(), first);
    assert.equal(publications, 1);
    snap({ ...cursor, sequence: 11 });
    snap({ ...cursor, subscription_revision: 2 });
    snap({ ...cursor, epoch: 'second' });
    assert.equal(publications, 4);
    implementation.receive({ t: 'error', id: sub.id, code: 'unavailable' });
    snap({ ...cursor, epoch: 'second' });
    assert.equal(publications, 6);
    assert.equal(view.data().error, undefined);
    view.dispose(); client.close();
});
