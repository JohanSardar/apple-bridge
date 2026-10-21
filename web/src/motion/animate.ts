import { sampleSpring, type Spring } from './spring';
export const reducedMotion = () => matchMedia('(prefers-reduced-motion: reduce)').matches;
export function springAnimation(update: (value: number) => void, from: number, target: number, spring: Spring, velocity = 0, finished?: () => void) {
    let state = { value: from, velocity }, start = performance.now(), origin = from, initialVelocity = velocity, destination = target, frame = 0, stopped = false;
    const tick = (now: number) => {
        if (stopped)
            return;
        state = sampleSpring(origin, destination, initialVelocity, (now - start) / 1000, spring);
        update(state.value);
        if (Math.abs(state.value - destination) < .001 && Math.abs(state.velocity) < .01) {
            stopped = true;
            update(destination);
            finished?.();
            return;
        }
        frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return { retarget(next: number, carriedVelocity?: number) { state = sampleSpring(origin, destination, initialVelocity, (performance.now() - start) / 1000, spring); origin = state.value; initialVelocity = carriedVelocity ?? state.velocity; destination = next; start = performance.now(); if (stopped) {
            stopped = false;
            frame = requestAnimationFrame(tick);
        } }, cancel() { stopped = true; cancelAnimationFrame(frame); }, get state() { return state; } };
}
export function crossfade(element: Element, entering = true) {
    return element.animate([{ opacity: entering ? 0 : 1 }, { opacity: entering ? 1 : 0 }], { duration: reducedMotion() ? 150 : 180, easing: 'ease-out' });
}
export function enterPane(element: HTMLElement, phone: boolean, direction = 1) {
    if (reducedMotion()) {
        const animation = crossfade(element);
        return () => animation.cancel();
    }
    const distance = phone ? element.clientWidth * direction : 8;
    const animation = springAnimation(value => { element.style.transform = phone ? `translateX(${value}px)` : `translateY(${value}px)`; element.style.opacity = String(1 - Math.min(1, Math.abs(value / distance)) * .15); }, distance, 0, { response: .35, damping: 1 }, 0, () => { element.style.transform = ''; element.style.opacity = ''; });
    return () => animation.cancel();
}
export function sharedElement(element: HTMLElement, mutate: () => void) {
    const before = element.getBoundingClientRect();
    mutate();
    const after = element.getBoundingClientRect();
    if (reducedMotion()) {
        crossfade(element);
        return () => { };
    }
    const dx = before.x - after.x, dy = before.y - after.y, sx = before.width / Math.max(1, after.width), sy = before.height / Math.max(1, after.height);
    element.style.transformOrigin = 'top left';
    const animation = springAnimation(progress => { element.style.transform = `translate(${dx * (1 - progress)}px,${dy * (1 - progress)}px) scale(${sx + (1 - sx) * progress},${sy + (1 - sy) * progress})`; }, 0, 1, { response: .42, damping: .82 }, 0, () => { element.style.transform = ''; element.style.transformOrigin = ''; });
    return () => { animation.cancel(); element.style.transform = ''; element.style.transformOrigin = ''; };
}
