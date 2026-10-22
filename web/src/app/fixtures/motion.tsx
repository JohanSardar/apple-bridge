import { onCleanup } from 'solid-js';
import { springAnimation, sharedElement, reducedMotion } from '../../motion/animate';
import { FixtureFrame, useFixtureStatus } from '../FixtureFrame';
function Motion() {
    const setStatus = useFixtureStatus();
    let animated!: HTMLDivElement;
    let animation: ReturnType<typeof springAnimation> | undefined;
    let cancelFlip: (() => void) | undefined;
    let expanded = false;
    onCleanup(() => { animation?.cancel(); cancelFlip?.(); });
    return <><button class="button" onClick={() => { if (reducedMotion()) { animated.animate([{ opacity: 0 }, { opacity: 1 }], { duration: 150 }); return; } animation?.cancel(); animation = springAnimation(value => { animated.style.transform = `translateX(${value}px)`; }, 0, 180, { response: .42, damping: .82 }, 0, () => setStatus('Settled')); }}>Animate spring</button><button class="button" onClick={() => animation?.retarget(0)}>Interrupt spring</button><button class="button" onClick={() => { cancelFlip?.(); cancelFlip = sharedElement(animated, () => { expanded = !expanded; animated.style.width = expanded ? '200px' : '80px'; animated.style.height = expanded ? '200px' : '80px'; }); }}>Shared element</button><div ref={animated} class="motion-fixture" style={{ width: '80px', height: '80px', background: 'var(--blue)', 'border-radius': '20px' }}/></>;
}
export default function MotionFixture() { return <FixtureFrame><Motion/></FixtureFrame>; }
