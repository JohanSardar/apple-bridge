import { test } from 'node:test';
import assert from 'node:assert/strict';
import { dateLabel } from './date-label.ts';

test('calendar dates reuse equivalent formats and preserve time zones and invalid-zone errors', () => {
    const Original = Intl.DateTimeFormat;
    let constructions = 0;
    Intl.DateTimeFormat = class extends Original {
        constructor(locale?: Intl.LocalesArgument, options?: Intl.DateTimeFormatOptions) {
            constructions++;
            super(locale, options);
        }
    } as typeof Intl.DateTimeFormat;
    try {
        const date = new Date('2026-10-31T01:00:00Z');
        const options = { dateStyle: 'long', timeStyle: 'short', timeZone: 'America/Chicago' } as const;
        assert.equal(dateLabel(date, options), new Original(undefined, options).format(date));
        assert.equal(dateLabel(date, { timeZone: options.timeZone, timeStyle: 'short', dateStyle: 'long' }), new Original(undefined, options).format(date));
        assert.equal(constructions, 1);
        assert.equal(dateLabel(date, { ...options, timeZone: 'Asia/Tokyo' }), new Original(undefined, { ...options, timeZone: 'Asia/Tokyo' }).format(date));
        assert.equal(constructions, 2);
        assert.throws(() => dateLabel(date, { timeZone: '[ab-test] invalid' }), RangeError);
    } finally { Intl.DateTimeFormat = Original; }
});
