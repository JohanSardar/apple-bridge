import { draftBounds, shifted } from './edit';
import { civil, day, timeLabel, type CalendarEntry } from './store';
import type { CalendarEdit } from './Editor';
import { reducedMotion, springAnimation } from '../motion/animate';
export function calendarEditing(element: HTMLElement, options: { height: () => number; events: () => CalendarEntry[]; writable: (entry?: CalendarEntry) => boolean; edit: (edit: CalendarEdit) => void; move: (event: PointerEvent, entry: CalendarEntry, args: Record<string, unknown>) => void }) {
    let active: { pointer: number; x: number; y: number; date: Date; minute: number; entry?: CalendarEntry; resizing: boolean; column: HTMLElement; ghost: HTMLElement } | undefined;
    let animation: ReturnType<typeof springAnimation> | undefined;
    let settlingGhost: HTMLElement | undefined;
    let ignoredClick = false, clickTimer: ReturnType<typeof setTimeout> | undefined;
    const minute = (event: PointerEvent, column: HTMLElement) => Math.max(0, Math.min(1425, Math.round((event.clientY - column.getBoundingClientRect().top) / options.height() * 4) * 15));
    const clear = (remove = true) => { if (remove) active?.ghost.remove(); active = undefined; };
    const down = (event: PointerEvent) => {
        if (active && event.pointerType === 'touch' && !event.isPrimary) { if (element.hasPointerCapture(active.pointer)) element.releasePointerCapture(active.pointer); clear(); }
        if (!event.isPrimary || event.button !== 0 || !(event.target instanceof Element)) return;
        const column = event.target.closest<HTMLElement>('.calendar-time-day');
        if (!column || active) return;
        const button = event.target.closest<HTMLElement>('[data-event-id]') ?? event.target.closest('.calendar-time-event')?.querySelector<HTMLElement>('[data-event-id]'), entry = options.events().find(entry => entry.event.id === button?.dataset.eventId);
        if (!options.writable(entry) || entry?.event.data._pending || !entry && event.pointerType === 'touch') return;
        const date = civil(column.dataset.date ?? '');
        if (!Number.isFinite(date.getTime())) return;
        animation?.cancel(); settlingGhost?.remove();
        const ghost = document.createElement('div'); ghost.className = 'calendar-drag-ghost'; ghost.setAttribute('aria-hidden', 'true');
        active = { pointer: event.pointerId, x: event.clientX, y: event.clientY, minute: minute(event, column), date, entry, resizing: Boolean(event.target.closest('[data-resize]')), column, ghost };
        event.stopPropagation();
    };
    const move = (event: PointerEvent) => {
        if (!active || active.pointer !== event.pointerId || Math.hypot(event.clientX - active.x, event.clientY - active.y) < 6 && !active.ghost.isConnected) return;
        event.preventDefault();
        if (!element.hasPointerCapture(event.pointerId)) element.setPointerCapture(event.pointerId);
        let column = active.column;
        const target = document.elementFromPoint(event.clientX, event.clientY)?.closest<HTMLElement>('.calendar-time-day');
        if (target && !active.resizing) column = target;
        const date = civil(column.dataset.date ?? ''), deltaDays = Math.round((Date.UTC(date.getFullYear(), date.getMonth(), date.getDate()) - Date.UTC(active.date.getFullYear(), active.date.getMonth(), active.date.getDate())) / 86400000);
        const offset = Math.round((event.clientY - active.y) / options.height() * 4) * 15;
        let start: Date, end: Date;
        if (active.entry) {
            const patch = shifted(active.entry.event.data, offset, active.resizing, deltaDays); if (!patch) return;
            start = new Date(patch.start); end = new Date(patch.end);
        } else { start = day(active.date); start.setMinutes(Math.min(active.minute, minute(event, active.column))); end = day(active.date); end.setMinutes(Math.max(active.minute + 15, minute(event, active.column) + 15)); }
        column.append(active.ghost);
        active.ghost.style.top = (start.getHours() * 60 + start.getMinutes()) / 60 * options.height() + 'px';
        active.ghost.style.height = Math.max(20, (end.getTime() - start.getTime()) / 3600000 * options.height()) + 'px';
        active.ghost.dataset.snapTop = active.ghost.style.top; active.ghost.dataset.snapHeight = active.ghost.style.height;
        const residual = event.clientY - active.y - Math.round((event.clientY - active.y) / options.height() * 4) * options.height() / 4;
        if (active.entry) {
            if (active.resizing) active.ghost.style.height = Math.max(20, parseFloat(active.ghost.style.height) + residual) + 'px';
            else active.ghost.style.top = parseFloat(active.ghost.style.top) + residual + 'px';
        }
        active.ghost.textContent = `${timeLabel(start)} – ${timeLabel(end)}`;
        active.ghost.dataset.start = start.toISOString(); active.ghost.dataset.end = end.toISOString();
    };
    const up = (event: PointerEvent) => {
        if (!active || active.pointer !== event.pointerId) return;
        let keepGhost = false;
        const gesture = active, moved = gesture.ghost.isConnected;
        if (event.type === 'pointerup' && moved) {
            ignoredClick = true; clearTimeout(clickTimer); clickTimer = setTimeout(() => { ignoredClick = false; }, 350);
            const args = { start: gesture.ghost.dataset.start!, end: gesture.ghost.dataset.end! };
            if (!gesture.entry) options.edit({ draft: draftBounds(new Date(args.start), new Date(args.end)) });
            else if (Array.isArray(gesture.entry.event.data.recurrence) && gesture.entry.event.data.recurrence.length) options.edit({ record: gesture.entry.event, draft: args });
            else options.move(event, gesture.entry, args);
            if (!reducedMotion() && gesture.entry) {
                keepGhost = true; settlingGhost = gesture.ghost;
                const top = parseFloat(gesture.ghost.style.top), height = parseFloat(gesture.ghost.style.height), targetTop = parseFloat(gesture.ghost.dataset.snapTop!), targetHeight = parseFloat(gesture.ghost.dataset.snapHeight!);
                animation = springAnimation(value => { gesture.ghost.style.top = top + (targetTop - top) * value + 'px'; gesture.ghost.style.height = height + (targetHeight - height) * value + 'px'; gesture.ghost.style.opacity = String(1 - value); }, 0, 1, { response: .3, damping: 1 }, 0, () => gesture.ghost.remove());
            }
        }
        clear(!keepGhost); if (element.hasPointerCapture(event.pointerId)) element.releasePointerCapture(event.pointerId);
    };
    const click = (event: MouseEvent) => { if (ignoredClick) { event.preventDefault(); event.stopImmediatePropagation(); ignoredClick = false; } };
    element.addEventListener('pointerdown', down); element.addEventListener('pointermove', move); element.addEventListener('pointerup', up); element.addEventListener('pointercancel', up); element.addEventListener('click', click, true);
    return () => { animation?.cancel(); settlingGhost?.remove(); clearTimeout(clickTimer); clear(); element.removeEventListener('pointerdown', down); element.removeEventListener('pointermove', move); element.removeEventListener('pointerup', up); element.removeEventListener('pointercancel', up); element.removeEventListener('click', click, true); };
}
