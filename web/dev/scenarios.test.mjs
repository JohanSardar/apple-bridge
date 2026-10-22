import assert from 'node:assert/strict';
import test from 'node:test';
import { unmappedTitles } from './scenarios.mjs';

test('unregistered subflows cannot disappear behind the exact scenario filter', () => {
  const report = { suites: [{ suites: [{ specs: [
    { title: 'WEB-MSG-NEW tokens keeps token focus', file: 'new.spec.ts', line: 1 },
    { title: 'WEB-MSG-NEW-TOKENS keeps token focus', file: 'new.spec.ts', line: 2 },
    { title: 'WEB-PERF-SHELL-INPUT measures input', file: 'shell.spec.ts', line: 3 },
    { title: 'WEB-REM-ACTIONS-columns keeps focus', file: 'reminders.spec.ts', line: 4 },
    { title: 'WEB-MSG-SEARCH-LIST searches its own fixture', file: 'search.spec.ts', line: 5 },
    { title: 'WEB-NOT-REGISTERED unknown scenario', file: 'unknown.spec.ts', line: 6 },
    { title: 'PERF-INPUT-MISSING unknown performance scenario', file: 'perf.spec.ts', line: 7 },
    { title: 'WEB-MSG-NEW--TOKENS malformed separator', file: 'new.spec.ts', line: 8 },
  ] }] }] };
  assert.deepEqual(unmappedTitles(report, new Set(['WEB-MSG-NEW', 'WEB-PERF-SHELL', 'WEB-REM-ACTIONS', 'WEB-MSG-SEARCH', 'WEB-MSG-SEARCH-LIST'])), [
    'new.spec.ts:2: WEB-MSG-NEW-TOKENS keeps token focus',
    'shell.spec.ts:3: WEB-PERF-SHELL-INPUT measures input',
    'reminders.spec.ts:4: WEB-REM-ACTIONS-columns keeps focus',
    'unknown.spec.ts:6: WEB-NOT-REGISTERED unknown scenario',
    'perf.spec.ts:7: PERF-INPUT-MISSING unknown performance scenario',
    'new.spec.ts:8: WEB-MSG-NEW--TOKENS malformed separator',
  ]);
});
