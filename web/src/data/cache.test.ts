import { test } from 'node:test';
import assert from 'node:assert/strict';
import { viewCache } from './cache.ts';

test('queued view snapshots persist the latest data and cursor once per key and owner', async () => {
    const saved: any[] = [];
    Object.assign(globalThis, {
        document: { hidden: true, removeEventListener() {} },
        cancelAnimationFrame() {},
        indexedDB: {
            open() {
                const request: any = {};
                queueMicrotask(() => {
                    request.result = {
                        transaction() {
                            const transaction: any = {};
                            transaction.objectStore = () => ({
                                put(value: unknown) {
                                    saved.push(structuredClone(value));
                                    setTimeout(() => transaction.oncomplete(), 0);
                                },
                                index: () => ({ getAllKeys() {
                                    const keys: any = { result: [] };
                                    queueMicrotask(() => keys.onsuccess());
                                    return keys;
                                } }),
                                delete() {},
                            });
                            return transaction;
                        },
                    };
                    request.onsuccess();
                });
                return request;
            },
        },
    });
    viewCache.enable('first-device');
    const pending: Promise<unknown>[] = [];
    for (let index = 0; index < 100; index++) pending.push(viewCache.write('thread', { older: index }, { epoch: 'epoch', seq: index }));
    pending.push(viewCache.write('other-thread', { older: 10 }));
    await Promise.all(pending);
    assert.equal(saved.length, 2);
    assert.equal(saved[0].data.older, 99);
    assert.equal(saved[0].cursor.seq, 99);
    assert.equal(saved[1].key, 'other-thread');
    const stale = viewCache.write('thread', { older: 200 });
    viewCache.disable();
    viewCache.enable('second-device');
    const current = viewCache.write('thread', { older: 300 });
    await Promise.all([stale, current]);
    assert.equal(saved.length, 3);
    assert.equal(saved[2].owner, 'second-device');
    assert.equal(saved[2].data.older, 300);
    await viewCache.write('thread', { older: 400 });
    assert.equal(saved.length, 4);
    assert.equal(saved[3].data.older, 400);
    viewCache.disable();
});
