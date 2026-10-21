import { reducedMotion, springAnimation, crossfade } from './animate';
export function drag(element: HTMLElement, handlers: {
    start?: (event: PointerEvent) => boolean;
    move: (x: number, y: number, velocityX: number, velocityY: number) => void;
    end: (x: number, y: number, velocityX: number, velocityY: number, cancelled: boolean) => void;
}) {
    let active = -1, startX = 0, startY = 0, lastX = 0, lastY = 0, lastTime = 0, vx = 0, vy = 0;
    const down = (event: PointerEvent) => { if (event.button !== 0 || active !== -1 || handlers.start?.(event) === false)
        return; active = event.pointerId; startX = lastX = event.clientX; startY = lastY = event.clientY; lastTime = event.timeStamp; vx = vy = 0; element.setPointerCapture(active); };
    const move = (event: PointerEvent) => { if (event.pointerId !== active)
        return; const dt = Math.max(1, event.timeStamp - lastTime); vx = (event.clientX - lastX) / dt * 1000; vy = (event.clientY - lastY) / dt * 1000; lastX = event.clientX; lastY = event.clientY; lastTime = event.timeStamp; handlers.move(lastX - startX, lastY - startY, vx, vy); };
    const end = (event: PointerEvent) => { if (event.pointerId !== active)
        return; const cancelled = event.type !== 'pointerup'; active = -1; if (event.timeStamp - lastTime > 80)
        vx = vy = 0; handlers.end(lastX - startX, lastY - startY, vx, vy, cancelled); };
    element.addEventListener('pointerdown', down);
    element.addEventListener('pointermove', move);
    element.addEventListener('pointerup', end);
    element.addEventListener('pointercancel', end);
    element.addEventListener('lostpointercapture', end);
    return () => { element.removeEventListener('pointerdown', down); element.removeEventListener('pointermove', move); element.removeEventListener('pointerup', end); element.removeEventListener('pointercancel', end); element.removeEventListener('lostpointercapture', end); };
}
export function swipeBack(element: HTMLElement, background: HTMLElement, enabled: () => boolean, back: () => void) {
    let animation: ReturnType<typeof springAnimation> | undefined;
    let direction = 1;
    const update = (x: number) => { const width = element.clientWidth; const progress = Math.min(1, Math.max(0, x / width)); element.style.transform = `translateX(${x * direction}px)`; background.style.transform = `translateX(${(progress - 1) * width * .3 * direction}px)`; background.style.opacity = String(.7 + progress * .3); };
    const clear = () => { element.style.transform = ''; background.style.transform = ''; background.style.opacity = ''; };
    const dispose = drag(element, { start: event => { direction = getComputedStyle(element).direction === 'rtl' ? -1 : 1; const rect = element.getBoundingClientRect(); if ((event.target instanceof HTMLElement && event.target.closest('button,a,input,select,textarea')) || !enabled() || (direction === 1 ? event.clientX - rect.left : rect.right - event.clientX) > 28)
            return false; animation?.cancel(); return true; }, move: x => { if (reducedMotion())
            return; update(Math.max(0, x * direction)); }, end: (x, _y, vx, _vy, cancelled) => { const commit = !cancelled && (x * direction > element.clientWidth * .35 || vx * direction > 600); if (reducedMotion()) {
            if (commit)
                back();
            crossfade(background);
            clear();
            return;
        } animation = springAnimation(update, Math.max(0, x * direction), commit ? element.clientWidth : 0, { response: .35, damping: 1 }, vx * direction, () => { if (commit)
            back(); clear(); }); } });
    return () => { dispose(); animation?.cancel(); clear(); };
}
