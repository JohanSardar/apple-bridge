import { onCleanup } from 'solid-js';
import { reducedMotion, springAnimation } from '../motion/animate';
export function reminderSwipe(element: HTMLElement, enabled: () => boolean, reveal: () => void) {
    let suppressClick = false;
    let pointer = -1, startX = 0, startY = 0, offset = 0, horizontal = false;
    let animation: ReturnType<typeof springAnimation> | undefined;
    const clear = () => { element.style.transform = ''; };
    const down = (event: PointerEvent) => {
        if (!event.isTrusted || event.pointerType !== 'touch' || !enabled() || pointer !== -1) return;
        if (event.target instanceof Element && event.target.closest('input,textarea,select,a,.reminder-complete-control,.reminder-row-actions')) return;
        const rect = element.getBoundingClientRect();
        if (event.clientX - rect.left < 32 || rect.right - event.clientX < 32) return;
        animation?.cancel();
        suppressClick = false;
        pointer = event.pointerId;
        startX = event.clientX;
        startY = event.clientY;
        offset = 0;
        horizontal = false;
    };
    const move = (event: PointerEvent) => {
        if (event.pointerId !== pointer) return;
        const x = event.clientX - startX, y = event.clientY - startY;
        if (!horizontal && Math.abs(y) > 12 && Math.abs(y) > Math.abs(x)) { pointer = -1; return; }
        if (!horizontal && Math.abs(x) > 12 && Math.abs(x) > Math.abs(y) * 1.5) {
            horizontal = true;
            element.setPointerCapture(pointer);
        }
        if (!horizontal) return;
        event.preventDefault();
        const direction = getComputedStyle(element).direction === 'rtl' ? -1 : 1;
        offset = Math.min(0, Math.max(-100, x * direction));
        if (!reducedMotion()) element.style.transform = `translateX(${offset * direction}px)`;
    };
    const end = (event: PointerEvent) => {
        if (event.pointerId !== pointer) return;
        const show = event.type === 'pointerup' && horizontal && offset < -50;
        suppressClick = horizontal;
        pointer = -1;
        if (show) reveal();
        if (reducedMotion()) { clear(); return; }
        const direction = getComputedStyle(element).direction === 'rtl' ? -1 : 1;
        animation = springAnimation(value => element.style.transform = `translateX(${value * direction}px)`, offset, 0, { response: .35, damping: 1 }, 0, clear);
    };
    const click = (event: MouseEvent) => { if (suppressClick) { suppressClick = false; event.preventDefault(); event.stopImmediatePropagation(); } };
    element.addEventListener('click', click, true);
    element.addEventListener('pointerdown', down);
    element.addEventListener('pointermove', move, { passive: false });
    element.addEventListener('pointerup', end);
    element.addEventListener('pointercancel', end);
    element.addEventListener('lostpointercapture', end);
    onCleanup(() => {
        animation?.cancel();
        element.removeEventListener('click', click, true);
        element.removeEventListener('pointerdown', down);
        element.removeEventListener('pointermove', move);
        element.removeEventListener('pointerup', end);
        element.removeEventListener('pointercancel', end);
        element.removeEventListener('lostpointercapture', end);
        clear();
    });
}
