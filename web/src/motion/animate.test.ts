import { test } from 'node:test';
import assert from 'node:assert/strict';
import { enterPane } from './animate.ts';
import { sampleSpring } from './spring.ts';

test('pane entry uses native animation with the analytic spring and preserves swipe interruption', async () => {
    const animations: { frames: Keyframe[]; options: KeyframeAnimationOptions; animation: any }[] = [];
    Object.assign(globalThis, {
        matchMedia: () => ({ matches: false }),
        requestAnimationFrame: () => { throw new Error('Pane entry must not schedule JavaScript frames'); },
        getComputedStyle: () => ({ transform: 'matrix(1,0,0,1,123,0)', opacity: '.95' }),
    });
    const element = { style: { transform: '', opacity: '' }, animate(frames: Keyframe[], options: KeyframeAnimationOptions) {
        let finish!: () => void;
        const finished = new Promise<void>(resolve => { finish = resolve; });
        const animation = { playState: 'running', finished, finish() { this.playState = 'finished'; finish(); }, cancel() { this.playState = 'idle'; } };
        animations.push({ frames, options, animation }); return animation;
    } } as unknown as HTMLElement;
    const cancel = enterPane(element, true, -1, 390);
    const { frames, options, animation } = animations[0];
    assert.equal(frames[0].transform, 'translateX(-390px)');
    assert.equal(frames.at(-1)?.transform, 'translateX(0px)');
    assert.equal(frames.at(-1)?.opacity, 1);
    const expected = sampleSpring(-390, 0, 0, .1, { response: .35, damping: 1 });
    assert.equal(frames[12].transform, `translateX(${expected.value}px)`);
    assert.ok(Number(options.duration) > 500 && Number(options.duration) < 2000);
    assert.equal(element.style.transform, 'translateX(0px)');
    cancel();
    assert.equal(animation.playState, 'idle');
    assert.equal(element.style.transform, 'matrix(1,0,0,1,123,0)');
    assert.equal(element.style.opacity, '.95');
    enterPane(element, false, 1, 0);
    assert.equal(element.style.transform, 'translateY(0px)');
    animation.finish();
    await Promise.resolve();
    assert.equal(element.style.transform, 'translateY(0px)');
    animations[1].animation.finish();
    await Promise.resolve();
    assert.equal(element.style.transform, '');
    assert.equal(element.style.opacity, '');
    element.style.transform = 'translateX(42px)'; element.style.opacity = '.8';
    Object.assign(globalThis, { matchMedia: () => ({ matches: true }) });
    const stopFade = enterPane(element, true, 1, 390);
    assert.equal(element.style.transform, '');
    assert.equal(element.style.opacity, '');
    stopFade();
    assert.equal(animations[2].animation.playState, 'idle');
});
