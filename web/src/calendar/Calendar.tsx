import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { calendarPreferences } from '../app/preferences';
import { AppMenu } from '../app/AppMenu';
import { registerShortcut } from '../app/keyboard';
import { auth } from '../auth/session';
import { router } from '../app/router';
import { Icon } from '../design/Icon';
import { reducedMotion, springAnimation } from '../motion/animate';
import { drag } from '../motion/gestures';
import type { ViewData } from '../data/types';
import { color, civil, dateKey, dateLabel, day, eventDate, eventsOn, month, range, text, timeLabel, useCalendarView, views, week, zone, type CalendarEntry, type CalendarPage, type CalendarView } from './store';
import { Inspector } from './Inspector';
import './calendar.css';
const names: Record<CalendarView, string> = { day: 'Day', week: 'Week', month: 'Month', year: 'Year', list: 'List' };
const weekdays = Array.from({ length: 7 }, (_, index) => dateLabel(new Date(2026, 10, 1 + index), { weekday: 'short' }));
function parts() { return router.route().path.split('/').filter(Boolean).map(value => { try { return decodeURIComponent(value); } catch { return ''; } }); }
export default function Calendar() {
    const routeView = () => views.includes(parts()[1] as CalendarView) ? parts()[1] as CalendarView : 'month';
    const routeDate = () => { const parsed = civil(parts()[2] ?? ''); return Number.isFinite(parsed.getTime()) ? parsed : day(new Date()); };
    const [view, setView] = createSignal<CalendarView>(routeView()), [date, setDate] = createSignal(routeDate());
    const [search, setSearch] = createSignal(''), [sidebar, setSidebar] = createSignal(matchMedia('(min-width:1024px)').matches), [now, setNow] = createSignal(new Date()), [hourHeight, setHourHeight] = createSignal(56);
    const hidden = () => new Set(calendarPreferences.hidden());
    const setHidden = (update: (previous: Set<string>) => Set<string>) => calendarPreferences.setHidden([...update(hidden())]);
    const eventID = createMemo(() => parts()[1] === 'event' ? parts()[2] : undefined);
    createEffect(() => { if (!eventID()) { setView(routeView()); setDate(routeDate()); } });
    const [calendarIDs, setCalendarIDs] = createSignal<string[]>([]);
    const bounds = createMemo(() => range(view(), date(), Boolean(search().trim())));
    const page = useCalendarView<CalendarPage>('calendar.range', () => ({ start: bounds()[0].toISOString(), end: bounds()[1].toISOString(), time_zone: zone, text: search().trim(), calendars: calendarIDs().filter(id => !hidden().has(id)) }));
    createEffect(() => { const ids = (page().calendars ?? []).map(calendar => calendar.id).sort(); if (page().calendars) setCalendarIDs(previous => previous.join(',') === ids.join(',') ? previous : ids); });
    const details = useCalendarView<ViewData>('record', () => eventID() ? { id: eventID() } : undefined);
    const calendars = createMemo(() => new Map((page().calendars ?? []).map(calendar => [calendar.id, calendar])));
    const events = createMemo(() => (page().events ?? []).filter(entry => !hidden().has(text(entry.event.data.calendar_id))));
    const selected = createMemo(() => details().records?.find(record => record.id === eventID() && record.collection === 'calendar.event') ?? events().find(entry => entry.event.id === eventID())?.event);
    const heading = createMemo(() => view() === 'day' ? dateLabel(date(), { weekday: 'long', month: 'long', day: 'numeric', year: 'numeric' }) : view() === 'year' ? String(date().getFullYear()) : view() === 'week' ? `${dateLabel(week(date()), { month: 'short', day: 'numeric' })} – ${dateLabel(day(week(date()), 6), { month: 'short', day: 'numeric', year: 'numeric' })}` : dateLabel(date(), { month: 'long', year: 'numeric' }));
    const eventColor = (entry: CalendarEntry) => color(calendars().get(text(entry.event.data.calendar_id))?.data.color);
    const navigate = (nextView: CalendarView, nextDate: Date) => { setSearch(''); if (innerWidth < 1024) setSidebar(false); router.navigate(`/calendar/${nextView}/${dateKey(nextDate)}`); };
    const move = (direction: number) => navigate(view(), view() === 'year' ? new Date(date().getFullYear() + direction, 0, 1) : view() === 'month' ? month(date(), direction) : day(date(), direction * (view() === 'week' ? 7 : view() === 'list' ? 90 : 1)));
    const open = (entry: CalendarEntry) => router.navigate('/calendar/event/' + encodeURIComponent(entry.event.id));
    const close = () => router.navigate(`/calendar/${view()}/${dateKey(date())}`, true);
    let content!: HTMLElement, cancelMotion: (() => void) | undefined;
    const animate = (from: number, velocity = 0) => {
        cancelMotion?.();
        if (reducedMotion()) { content.style.transform = ''; return; }
        const animation = springAnimation(value => { content.style.transform = `translateX(${value}px)`; }, from, 0, { response: .35, damping: 1 }, velocity, () => { content.style.transform = ''; });
        cancelMotion = () => animation.cancel();
    };
    const keydown = (event: KeyboardEvent) => {
        if (event.isComposing || event.metaKey || event.ctrlKey || event.altKey || (event.target instanceof HTMLElement && event.target.closest('input,textarea,select,button,a,[contenteditable=true]'))) return;
        if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') { event.preventDefault(); move(event.key === 'ArrowLeft' ? -1 : 1); }
        if (event.key.toLowerCase() === 't') { event.preventDefault(); navigate(view(), new Date()); }
        if (event.key === 'Escape') { setSearch(''); setSidebar(false); if (eventID()) close(); }
    };
    onMount(() => {
        const timer = setInterval(() => setNow(new Date()), 60000);
        const shortcuts = views.slice(0, 4).map((value, index) => registerShortcut({ key: String(index + 1), action: () => navigate(value, date()) }));
        shortcuts.push(registerShortcut({ key: 't', action: () => navigate(view(), new Date()) }), registerShortcut({ key: 'ArrowLeft', action: () => move(-1) }), registerShortcut({ key: 'ArrowRight', action: () => move(1) }));
        shortcuts.push(registerShortcut({ key: 'f', input: true, action: () => document.querySelector<HTMLInputElement>('.calendar-search')?.focus() }));
        window.addEventListener('keydown', keydown);
        let axis: 'x' | 'y' | undefined;
        const dispose = drag(content, {
            start: event => { if (event.pointerType !== 'touch' || (event.target instanceof HTMLElement && event.target.closest('button,a,input,select'))) return false; cancelMotion?.(); axis = undefined; return true; },
            move: (x, y) => { if (!axis && Math.max(Math.abs(x), Math.abs(y)) > 10) axis = Math.abs(x) > Math.abs(y) * 1.3 ? 'x' : 'y'; if (axis === 'x' && !reducedMotion()) content.style.transform = `translateX(${x * .65}px)`; },
            end: (x, _y, velocity, _vy, cancelled) => { if (axis === 'x' && !cancelled && (Math.abs(x) > content.clientWidth * .2 || Math.abs(velocity) > 550)) { move(x < 0 ? 1 : -1); animate(x < 0 ? content.clientWidth * .35 : -content.clientWidth * .35, velocity * .2); } else animate(axis === 'x' ? x * .65 : 0); }
        });
        const wheel = (event: WheelEvent) => { if (!event.ctrlKey || !['day', 'week'].includes(view())) return; event.preventDefault(); setHourHeight(value => Math.max(32, Math.min(120, value - event.deltaY * .1))); };
        content.addEventListener('wheel', wheel, { passive: false });
        const pointers = new Map<number, { x: number; y: number }>();
        let pinchDistance = 0, pinchHeight = 0;
        const pinch = (event: PointerEvent) => {
            if (event.pointerType !== 'touch') return;
            if (event.type === 'pointerup' || event.type === 'pointercancel') { pointers.delete(event.pointerId); pinchDistance = 0; return; }
            pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
            if (pointers.size !== 2 || !['day', 'week'].includes(view())) return;
            axis = 'y';
            const [a, b] = [...pointers.values()], distance = Math.hypot(a.x - b.x, a.y - b.y);
            if (!pinchDistance) { pinchDistance = distance; pinchHeight = hourHeight(); }
            else if (pinchDistance > 0) { event.preventDefault(); setHourHeight(Math.max(32, Math.min(120, pinchHeight * distance / pinchDistance))); }
        };
        for (const type of ['pointerdown', 'pointermove', 'pointerup', 'pointercancel']) content.addEventListener(type, pinch as EventListener, { passive: false });
        onCleanup(() => { clearInterval(timer); shortcuts.forEach(dispose => dispose()); window.removeEventListener('keydown', keydown); content.removeEventListener('wheel', wheel); for (const type of ['pointerdown', 'pointermove', 'pointerup', 'pointercancel']) content.removeEventListener(type, pinch as EventListener); dispose(); cancelMotion?.(); });
    });
    createEffect(() => {
        const active = view(); date();
        if (active === 'day' || active === 'week') queueMicrotask(() => { const hours = content?.querySelector<HTMLElement>('.calendar-hours'); if (hours) hours.scrollTop = 7 * hourHeight(); });
    });
    function EventButton(props: { entry: CalendarEntry; compact?: boolean }) {
        return <button class={'calendar-event' + (props.entry.event.data.all_day ? ' all-day' : '')} classList={{ tentative: props.entry.event.data.status === 2, cancelled: props.entry.event.data.status === 3 }} style={{ '--event-color': eventColor(props.entry) }} data-event-id={props.entry.event.id} onClick={() => open(props.entry)} title={text(props.entry.event.data.title)}><Show when={!props.entry.event.data.all_day && !props.compact}><time>{timeLabel(eventDate(props.entry, 'start'))}</time></Show><span>{text(props.entry.event.data.title) || 'Untitled event'}</span><Show when={Array.isArray(props.entry.event.data.recurrence) && (props.entry.event.data.recurrence as unknown[]).length}><small aria-hidden="true" title="Repeats">↻</small></Show></button>;
    }
    function MiniMonth(props: { date: Date; year?: boolean }) {
        const cells = () => Array.from({ length: 42 }, (_, index) => day(week(month(props.date)), index));
        return <section class={'calendar-mini' + (props.year ? ' year-mini' : '')} aria-label={dateLabel(props.date, { month: 'long', year: 'numeric' })}><button class="calendar-mini-title" onClick={() => navigate('month', props.date)}>{dateLabel(props.date, { month: 'long', ...(props.year ? {} : { year: 'numeric' }) })}</button><div class="calendar-mini-grid"><For each={weekdays}>{label => <span class="calendar-weekday">{label.slice(0, 1)}</span>}</For><For each={cells()}>{cell => <button classList={{ outside: cell.getMonth() !== props.date.getMonth(), today: dateKey(cell) === dateKey(now()), selected: dateKey(cell) === dateKey(date()) }} aria-label={dateLabel(cell)} onClick={() => navigate('day', cell)}>{cell.getDate()}<Show when={eventsOn(events(), cell).length}><i/></Show></button>}</For></div></section>;
    }
    function Month() {
        const cells = () => Array.from({ length: 42 }, (_, index) => day(bounds()[0], index));
        return <><div class="calendar-weekdays"><For each={weekdays}>{label => <span>{label}</span>}</For></div><div class="calendar-month" role="grid" aria-label={heading()}><For each={cells()}>{cell => <div class="calendar-day-cell" role="gridcell" classList={{ outside: cell.getMonth() !== date().getMonth(), today: dateKey(cell) === dateKey(now()) }}><button class="calendar-day-number" aria-label={dateLabel(cell)} onClick={() => navigate(innerWidth < 1024 ? 'month' : 'day', cell)}>{cell.getDate()}</button><div class="calendar-cell-events"><For each={eventsOn(events(), cell).slice(0, 4)}>{entry => <EventButton entry={entry}/>}</For><Show when={eventsOn(events(), cell).length > 4}><button class="calendar-more" onClick={() => navigate('day', cell)}>+{eventsOn(events(), cell).length - 4} more</button></Show></div><div class="calendar-dots" aria-hidden="true"><For each={eventsOn(events(), cell).slice(0, 3)}>{entry => <i style={{ background: eventColor(entry) }}/>}</For></div></div>}</For></div><section class="calendar-month-agenda" aria-label="Selected day events"><h2>{dateLabel(date(), { weekday: 'long', month: 'long', day: 'numeric' })}</h2><For each={eventsOn(events(), date())}>{entry => <EventButton entry={entry}/>}</For><Show when={page().events && !eventsOn(events(), date()).length}><p>No events</p></Show></section></>;
    }
    function Timeline() {
        const days = () => Array.from({ length: view() === 'week' ? 7 : 1 }, (_, index) => day(bounds()[0], index));
        const placement = (entry: CalendarEntry, date: Date) => {
            const start = eventDate(entry, 'start'), end = eventDate(entry, 'end');
            const from = start < day(date) ? 0 : start.getHours() * 60 + start.getMinutes();
            const to = end >= day(date, 1) ? 1440 : end.getHours() * 60 + end.getMinutes();
            return { top: `${from / 60 * hourHeight()}px`, height: `${Math.max(20, (to - from) / 60 * hourHeight())}px`, left: `${entry.column / entry.columns * 100}%`, width: `${100 / entry.columns}%` };
        };
        return <div class="calendar-timeline" style={{ '--day-count': days().length, '--hour-height': hourHeight() + 'px' }}><div class="calendar-time-header"><span>{zone}</span><For each={days()}>{cell => <button onClick={() => navigate('day', cell)} classList={{ today: dateKey(cell) === dateKey(now()) }}>{dateLabel(cell, { weekday: 'short', day: 'numeric' })}</button>}</For></div><div class="calendar-all-day"><span>All-day</span><For each={days()}>{cell => <div><For each={eventsOn(events(), cell).filter(entry => entry.event.data.all_day)}>{entry => <EventButton entry={entry} compact/>}</For></div>}</For></div><div class="calendar-hours" data-scroll="calendar-hours"><div class="calendar-hour-labels"><For each={Array.from({ length: 24 }, (_, hour) => hour)}>{hour => <span style={{ top: hour * hourHeight() + 'px' }}>{timeLabel(new Date(2026, 0, 1, hour))}</span>}</For></div><For each={days()}>{cell => <div class="calendar-time-day" style={{ height: 24 * hourHeight() + 'px' }}><For each={eventsOn(events(), cell).filter(entry => !entry.event.data.all_day)}>{entry => <div class="calendar-time-event" style={placement(entry, cell)}><EventButton entry={entry}/></div>}</For><Show when={dateKey(cell) === dateKey(now())}><div class="calendar-now" aria-label="Current time" style={{ transform: `translateY(${(now().getHours() + now().getMinutes() / 60) * hourHeight()}px)` }}><i/></div></Show></div>}</For></div></div>;
    }
    function List() {
        const groups = createMemo(() => {
            const byDate = new Map<string, CalendarEntry[]>();
            for (const entry of events()) { const key = dateKey(eventDate(entry, 'start')); const items = byDate.get(key) ?? []; items.push(entry); byDate.set(key, items); }
            return [...byDate.entries()];
        });
        return <div class="calendar-list" data-scroll="calendar-list"><Show when={groups().length} fallback={<p class="calendar-empty">{page().events ? 'No events' : 'Loading events…'}</p>}><For each={groups()}>{([key, entries]) => <section><h2>{dateLabel(civil(key), { weekday: 'long', month: 'long', day: 'numeric' })}</h2><For each={entries}>{entry => <div class="calendar-list-row"><span>{entry.event.data.all_day ? 'All-day' : timeLabel(eventDate(entry, 'start'))}</span><EventButton entry={entry} compact/><span>{text(entry.event.data.location)}</span></div>}</For></section>}</For></Show></div>;
    }
    return <main class="calendar-app" classList={{ 'sidebar-hidden': !sidebar() }} aria-label="Calendar"><aside class="calendar-sidebar" classList={{ visible: sidebar() }}><header><AppMenu/><h1>Calendar</h1><button class="icon-button calendar-sidebar-close" aria-label="Close calendars" onClick={() => setSidebar(false)}><Icon name="close"/></button></header><label class="calendar-search-wrap"><Icon name="search"/><input class="calendar-search" type="search" placeholder="Search events" aria-label="Search events" value={search()} onInput={event => setSearch(event.currentTarget.value)}/></label><MiniMonth date={date()}/><h2>Calendars</h2><For each={page().calendars ?? []}>{calendar => <label class="calendar-toggle"><input type="checkbox" checked={!hidden().has(calendar.id)} style={{ 'accent-color': color(calendar.data.color) }} onChange={event => setHidden(previous => { const next = new Set(previous); if (event.currentTarget.checked) next.delete(calendar.id); else next.add(calendar.id); return next; })}/><span>{text(calendar.data.title)}</span><Show when={calendar.data.writable === false}><small>Read-only</small></Show></label>}</For><p class="calendar-zone">{zone}</p></aside><section class="calendar-main"><header class="calendar-toolbar"><button class="icon-button" aria-label="Show calendars" aria-expanded={sidebar()} onClick={() => setSidebar(!sidebar())}><Icon name="list"/></button><div class="calendar-views" role="group" aria-label="Calendar view"><For each={views}>{value => <button aria-pressed={view() === value} onClick={() => navigate(value, date())}>{names[value]}</button>}</For></div><button class="calendar-today" onClick={() => navigate(view(), new Date())}>Today</button><div class="calendar-page-buttons"><button aria-label="Previous period" onClick={() => move(-1)}>‹</button><button aria-label="Next period" onClick={() => move(1)}>›</button></div></header><Show when={auth.live()?.status() !== 'connected'}><p class="calendar-notice" role="status">Reconnecting…</p></Show><div class="calendar-heading"><h1 aria-live="polite">{search().trim() ? 'Search results' : heading()}</h1><Show when={search().trim()}><button onClick={() => setSearch('')}>Done</button></Show></div><section class="calendar-content" classList={{ 'month-mode': view() === 'month' && !search().trim() }} ref={content} tabindex="0" aria-label="Events"><Show when={!page().error} fallback={<p class="calendar-empty" role="alert">{page().error === 'capacity' ? 'Too many events in this period. Choose a shorter view.' : 'Calendar unavailable. Try again.'} <button onClick={() => location.reload()}>Reload</button></p>}><Show when={!search().trim()} fallback={<List/>}><Show when={view() === 'month'}><Month/></Show><Show when={view() === 'day' || view() === 'week'}><Timeline/></Show><Show when={view() === 'year'}><div class="calendar-year"><For each={Array.from({ length: 12 }, (_, index) => new Date(date().getFullYear(), index, 1))}>{value => <MiniMonth date={value} year/>}</For></div></Show><Show when={view() === 'list'}><List/></Show></Show></Show></section></section><Show when={eventID()}><Inspector record={selected()} calendar={calendars().get(text(selected()?.data.calendar_id))} close={close}/></Show></main>;
}
