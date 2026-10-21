import { test } from 'node:test';
import assert from 'node:assert/strict';
import { sampleSpring, rubberBand } from './spring.ts';
for (const damping of [.5, 1, 1.5])
    test(`analytic spring preserves initial conditions and converges (${damping})`, () => { const initial = sampleSpring(3, 8, 7, 0, { response: .4, damping }); assert.ok(Math.abs(initial.value - 3) < 1e-10); assert.ok(Math.abs(initial.velocity - 7) < 1e-10); const settled = sampleSpring(3, 8, 7, 10, { response: .4, damping }); assert.ok(Math.abs(settled.value - 8) < 1e-8); assert.ok(Math.abs(settled.velocity) < 1e-8); });
test('velocity matches independent numerical derivative and interruption is continuous', () => { const spring = { response: .42, damping: .82 }, t = .137, dt = 1e-6; const state = sampleSpring(0, 100, 30, t, spring); const derivative = (sampleSpring(0, 100, 30, t + dt, spring).value - sampleSpring(0, 100, 30, t - dt, spring).value) / (2 * dt); assert.ok(Math.abs(state.velocity - derivative) < 1e-5); const interrupted = sampleSpring(state.value, -50, state.velocity, 0, spring); assert.ok(Math.abs(interrupted.value - state.value) < 1e-10); assert.ok(Math.abs(interrupted.velocity - state.velocity) < 1e-10); });
test('rubber band is bounded, symmetric and monotonic', () => { let previous = 0; for (let x = 0; x < 10000; x += 17) {
    const result = rubberBand(x, 390);
    assert.ok(result >= previous && result < 390);
    assert.equal(result, -rubberBand(-x, 390));
    previous = result;
} });
