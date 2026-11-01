import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createFontWatcher } from './fonts.ts';

test('Unicode-only fallback font loads invalidate measurements without shaping a sentinel', () => {
    const fonts = new EventTarget();
    Object.assign(globalThis, { document: { fonts } });
    let changes = 0;
    const watcher = createFontWatcher(() => changes++);
    for (const family of ['CJK fallback', 'Arabic fallback', 'Color emoji']) {
        const event = new Event('loadingdone');
        Object.assign(event, { fontfaces: [{ family }] });
        fonts.dispatchEvent(event);
    }
    assert.equal(changes, 3);
    watcher.resized({ width: 30, height: 20 } as DOMRectReadOnly);
    assert.equal(changes, 3);
    watcher.resized({ width: 32, height: 20 } as DOMRectReadOnly);
    assert.equal(changes, 4);
    watcher.dispose(); fonts.dispatchEvent(new Event('loadingdone'));
    watcher.resized({ width: 34, height: 20 } as DOMRectReadOnly);
    assert.equal(changes, 4);
});
