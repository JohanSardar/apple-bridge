import { reducedMotion, springAnimation } from '../motion/animate';
import { drag } from '../motion/gestures';
import type { CalendarView } from './store';
type Axis = 'x' | 'y';
export function calendarNavigation(element: HTMLElement, options: { view: () => CalendarView; page: (direction: number) => void; height: () => number; setHeight: (height: number) => void }) {
    let animation: ReturnType<typeof springAnimation> | undefined, wheelTimer: ReturnType<typeof setTimeout> | undefined;
    let axis: Axis = 'x', intent: Axis | 'scroll' | undefined, wheelDistance = 0, wheelVelocity = 0, wheelAt = 0;
    const clear = () => { element.style.transform = ''; element.style.transformOrigin = ''; element.style.opacity = ''; };
    const cancel = () => { animation?.cancel(); animation = undefined; clear(); };
    const size = (axis: Axis) => axis === 'x' ? element.clientWidth : element.clientHeight;
    const position = (distance: number, axis: Axis) => { if (!reducedMotion()) element.style.transform = `translate${axis.toUpperCase()}(${Math.max(-size(axis) * .7, Math.min(size(axis) * .7, distance))}px)`; };
    const settle = (distance: number, velocity: number, axis: Axis) => {
        cancel();
        if (reducedMotion()) return;
        animation = springAnimation(value => position(value, axis), distance, 0, { response: .35, damping: 1 }, Math.max(-2000, Math.min(2000, velocity)), clear);
    };
    const finish = (distance: number, velocity: number, axis: Axis, cancelled: boolean) => {
        const commit = !cancelled && (Math.abs(distance) > size(axis) * .18 || Math.abs(velocity) > 550);
        if (commit) { options.page(distance < 0 ? 1 : -1); settle((distance < 0 ? 1 : -1) * size(axis) * .3, velocity * .2, axis); }
        else settle(distance * .65, velocity, axis);
    };
    const compactMonth = (target: EventTarget | null) => innerWidth < 1024 && options.view() === 'month' && target instanceof Element && Boolean(target.closest('.calendar-month'));
    const disposeDrag = drag(element, {
        start: event => {
            if (event.pointerType !== 'touch' || (event.target instanceof Element && event.target.closest('button,a,input,select,textarea,[data-resize]'))) return false;
            clearTimeout(wheelTimer); wheelDistance = 0; cancel(); intent = undefined; axis = compactMonth(event.target) ? 'y' : 'x'; return true;
        },
        move: (x, y) => {
            if (!intent && Math.max(Math.abs(x), Math.abs(y)) > 10) intent = axis === 'y' ? Math.abs(y) > Math.abs(x) * 1.2 ? 'y' : 'scroll' : Math.abs(x) > Math.abs(y) * 1.2 ? 'x' : 'scroll';
            if (intent === 'x' || intent === 'y') position((intent === 'x' ? x : y) * .65, intent);
        },
        end: (x, y, vx, vy, cancelled) => { if (intent === 'x' || intent === 'y') finish(intent === 'x' ? x : y, intent === 'x' ? vx : vy, intent, cancelled); else cancel(); }
    });
    const wheel = (event: WheelEvent) => {
        if (event.ctrlKey && ['day', 'week'].includes(options.view())) { event.preventDefault(); options.setHeight(Math.max(32, Math.min(120, options.height() - event.deltaY * .1))); return; }
        if (event.ctrlKey) return;
        const horizontal = Math.abs(event.deltaX) > Math.abs(event.deltaY) * 1.2;
        const vertical = compactMonth(event.target) && Math.abs(event.deltaY) > Math.abs(event.deltaX) * 1.2;
        if (!horizontal && !vertical) return;
        event.preventDefault();
        const nextAxis = vertical ? 'y' : 'x', multiplier = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? size(nextAxis) : 1;
        const delta = -(vertical ? event.deltaY : event.deltaX) * multiplier;
        const elapsed = wheelTimer ? Math.max(16, event.timeStamp - wheelAt) : 16;
        if (!wheelTimer || axis !== nextAxis) { cancel(); wheelDistance = 0; }
        axis = nextAxis; wheelDistance += delta; wheelVelocity = delta / elapsed * 1000; wheelAt = event.timeStamp;
        position(wheelDistance * .65, axis);
        clearTimeout(wheelTimer);
        wheelTimer = setTimeout(() => { const distance = wheelDistance, velocity = wheelVelocity, capturedAxis = axis; wheelTimer = undefined; wheelDistance = 0; finish(distance, velocity, capturedAxis, false); }, 120);
    };
    element.addEventListener('wheel', wheel, { passive: false });
    const pointers = new Map<number, { x: number; y: number }>();
    let pinchDistance = 0, pinchHeight = 0;
    const pinch = (event: PointerEvent) => {
        if (event.pointerType !== 'touch') return;
        if (event.type === 'pointerup' || event.type === 'pointercancel') { pointers.delete(event.pointerId); pinchDistance = 0; return; }
        pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
        if (pointers.size !== 2 || !['day', 'week'].includes(options.view())) return;
        intent = 'scroll'; cancel();
        const [a, b] = [...pointers.values()], distance = Math.hypot(a.x - b.x, a.y - b.y);
        if (!pinchDistance) { pinchDistance = distance; pinchHeight = options.height(); }
        else { event.preventDefault(); options.setHeight(Math.max(32, Math.min(120, pinchHeight * distance / pinchDistance))); }
    };
    for (const type of ['pointerdown', 'pointermove', 'pointerup', 'pointercancel']) element.addEventListener(type, pinch as EventListener, { passive: false });
    return {
        zoomFrom(anchor: DOMRect) {
            clearTimeout(wheelTimer); wheelTimer = undefined; cancel();
            if (reducedMotion()) return;
            const rect = element.getBoundingClientRect();
            element.style.transformOrigin = `${anchor.x + anchor.width / 2 - rect.x}px ${anchor.y + anchor.height / 2 - rect.y}px`;
            animation = springAnimation(value => { element.style.transform = `scale(${.85 + value * .15})`; element.style.opacity = String(.5 + value * .5); }, 0, 1, { response: .35, damping: .9 }, 0, clear);
        },
        dispose() { clearTimeout(wheelTimer); disposeDrag(); element.removeEventListener('wheel', wheel); for (const type of ['pointerdown', 'pointermove', 'pointerup', 'pointercancel']) element.removeEventListener(type, pinch as EventListener); cancel(); }
    };
}
