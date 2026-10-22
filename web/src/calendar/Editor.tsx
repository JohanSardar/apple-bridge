import { For, Show, createSignal } from 'solid-js';
import type { BridgeRecord } from '../data/types';
import { civil, dateKey, day, object, text, zone } from './store';
import { instant, wallTime } from './edit';
type Draft = Record<string, unknown>;
export type CalendarEdit = { record?: BridgeRecord; draft?: Draft; deleting?: boolean; calendar?: boolean };
export function Editor(props: { edit: CalendarEdit; calendars: BridgeRecord[]; close: () => void; save: (event: Event, op: string, args: Draft, before?: BridgeRecord) => boolean }) {
    const baseline = props.edit.record?.data ?? {}, initial = { ...baseline, ...props.edit.draft }, editing = Boolean(props.edit.record), recurring = Array.isArray(initial.recurrence) && initial.recurrence.length > 0;
    const [error, setError] = createSignal(''), [allDay, setAllDay] = createSignal(Boolean(initial.all_day)), [timeZone, setTimeZone] = createSignal(text(initial.time_zone) || zone);
    const display = (field: string) => initial.all_day ? text(initial[field]) : wallTime(text(initial[field]), timeZone());
    const initialStart = display('start'), initialEnd = initial.all_day ? dateKey(day(civil(text(initial.end)), -1)) : display('end'), initialZone = timeZone();
    const [start, setStart] = createSignal(initialStart), [end, setEnd] = createSignal(initialEnd);
    const [title, setTitle] = createSignal(text(initial.title)), [calendarID, setCalendarID] = createSignal(text(initial.calendar_id) || props.calendars.find(calendar => calendar.data.writable === true)?.id || '');
    const [notes, setNotes] = createSignal(text(initial.notes)), [location, setLocation] = createSignal(text(initial.location)), [url, setURL] = createSignal(text(initial.url));
    const [availability, setAvailability] = createSignal(String(initial.availability ?? 0)), [span, setSpan] = createSignal('this');
    const [repeat, setRepeat] = createSignal(editing ? 'keep' : ''), [interval, setInterval] = createSignal('1'), [ending, setEnding] = createSignal('never'), [count, setCount] = createSignal('2'), [until, setUntil] = createSignal('');
    const [alertsChanged, setAlertsChanged] = createSignal(false);
    const alarms = Array.isArray(initial.alarms) ? initial.alarms.map(object) : [];
    const [alerts, setAlerts] = createSignal(alarms.slice(0, 2).map(alarm => typeof alarm.relative_offset === 'number' ? String(-alarm.relative_offset / 60) : 'absolute'));
    const [absolute, setAbsolute] = createSignal(alarms.slice(0, 2).map(alarm => text(alarm.absolute_date) ? wallTime(text(alarm.absolute_date), timeZone()) : ''));
    const changeAllDay = (checked: boolean) => {
        if (checked) { setStart(start().slice(0, 10)); setEnd(end().slice(0, 10)); }
        else { setStart(start().slice(0, 10) + 'T09:00'); setEnd(end().slice(0, 10) + 'T10:00'); }
        setAllDay(checked);
    };
    const save = (event: Event) => {
        try {
            if (props.edit.calendar) {
                if (!title().trim()) throw new Error('Enter a calendar name.');
                if (!props.save(event, 'calendar.create_calendar', { title: title().trim() })) throw new Error('Unable to save. Check your connection and available actions.'); props.close(); return;
            }
            const args: Draft = editing ? { id: props.edit.record!.id, span: span() } : {};
            if (props.edit.deleting) { if (!props.save(event, 'calendar.delete_event', args, props.edit.record)) throw new Error('Unable to save. Check your connection and available actions.'); props.close(); return; }
            if (!title().trim() || !calendarID()) throw new Error('Enter a title and choose a writable calendar.');
            const sameMode = allDay() === Boolean(initial.all_day) && timeZone() === initialZone;
            const starts = sameMode && start() === initialStart ? text(initial.start) : allDay() ? start() : instant(start(), timeZone()), ends = sameMode && end() === initialEnd ? text(initial.end) : allDay() ? dateKey(day(civil(end()), 1)) : instant(end(), timeZone());
            if (!Number.isFinite(Date.parse(starts)) || !Number.isFinite(Date.parse(ends)) || ends <= starts) throw new Error('The event must end after it starts.');
            const values: Draft = { title: title().trim(), calendar_id: calendarID(), notes: notes(), location: location(), url: url(), availability: Number(availability()), start: starts, end: ends, all_day: allDay(), time_zone: allDay() ? '' : timeZone() };
            for (const [key, value] of Object.entries(values)) if (!editing || value !== baseline[key] && !(value === '' && baseline[key] == null)) args[key] = value;
            if (args.all_day !== undefined) { args.start = starts; args.end = ends; args.time_zone = values.time_zone; }
            if (repeat() !== 'keep') {
                args.recurrence = [];
                if (repeat()) {
                    const rule: Draft = { frequency: repeat(), interval: Number(interval()) };
                    if (!Number.isInteger(rule.interval) || Number(rule.interval) < 1) throw new Error('Repeat interval must be a positive whole number.');
                    if (ending() === 'count') { rule.count = Number(count()); if (!Number.isInteger(rule.count) || Number(rule.count) < 1) throw new Error('Enter a positive repeat count.'); }
                    if (ending() === 'date') { rule.end = allDay() ? until() + 'T00:00:00Z' : instant(until() + 'T' + start().slice(11), timeZone()); if (text(rule.end) < starts) throw new Error('Repeat must end on or after the first event.'); }
                    args.recurrence = [rule];
                }
            }
            if (alertsChanged() || !editing) args.alarms = alerts().map((alert, index) => alert === 'absolute' ? { absolute_date: instant(absolute()[index], timeZone()) } : { relative_offset: -Number(alert) * 60 });
            if (editing && Object.keys(args).length === 2) { props.close(); return; }
            if (!props.save(event, editing ? 'calendar.update_event' : 'calendar.create_event', args, props.edit.record)) throw new Error('Unable to save. Check your connection and available actions.'); props.close();
        } catch (failure) { setError(failure instanceof Error ? failure.message : 'Check the event details.'); }
    };
    return <dialog class="calendar-editor" aria-label={props.edit.calendar ? 'New calendar' : props.edit.deleting ? 'Delete event' : editing ? 'Edit event' : 'New event'} ref={element => queueMicrotask(() => element.showModal())} onCancel={props.close}>
        <header><h2>{props.edit.calendar ? 'New calendar' : props.edit.deleting ? 'Delete event?' : editing ? 'Edit event' : 'New event'}</h2><button type="button" onClick={props.close}>Cancel</button></header>
        <form onSubmit={event => event.preventDefault()} onKeyDown={event => { if (event.key === 'Enter' && event.target instanceof HTMLInputElement && event.target.type !== 'checkbox') { event.preventDefault(); save(event); } }}>
            <Show when={!props.edit.deleting} fallback={<p>Delete “{text(initial.title)}”?</p>}>
                <label>Title<input autofocus required value={title()} onInput={event => setTitle(event.currentTarget.value)}/></label>
                <Show when={!props.edit.calendar}>
                    <label>Calendar<select value={calendarID()} onChange={event => setCalendarID(event.currentTarget.value)}><For each={props.calendars.filter(calendar => calendar.data.writable === true)}>{calendar => <option value={calendar.id}>{text(calendar.data.title)}</option>}</For></select></label>
                    <label class="calendar-check"><input type="checkbox" checked={allDay()} onChange={event => changeAllDay(event.currentTarget.checked)}/>All-day</label>
                    <div class="calendar-date-fields"><label>Starts<input type={allDay() ? 'date' : 'datetime-local'} value={start()} onInput={event => setStart(event.currentTarget.value)}/></label><label>Ends<input type={allDay() ? 'date' : 'datetime-local'} value={end()} onInput={event => setEnd(event.currentTarget.value)}/></label></div>
                    <Show when={!allDay()}><label>Time zone<input list="calendar-time-zones" value={timeZone()} onInput={event => setTimeZone(event.currentTarget.value)}/><datalist id="calendar-time-zones"><For each={[zone, 'UTC', 'America/New_York', 'America/Los_Angeles', 'Europe/London', 'Asia/Tokyo']}>{value => <option value={value}/>}</For></datalist></label></Show>
                    <label>Repeat<select value={repeat()} onChange={event => setRepeat(event.currentTarget.value)}><Show when={editing}><option value="keep">Keep existing repeat</option></Show><option value="">Never</option><option value="daily">Daily</option><option value="weekly">Weekly</option><option value="monthly">Monthly</option><option value="yearly">Yearly</option></select></label>
                    <Show when={repeat() && repeat() !== 'keep'}><label>Every<input type="number" min="1" step="1" value={interval()} onInput={event => setInterval(event.currentTarget.value)}/></label><label>End repeat<select value={ending()} onChange={event => setEnding(event.currentTarget.value)}><option value="never">Never</option><option value="count">After a number of times</option><option value="date">On date</option></select></label><Show when={ending() === 'count'}><label>Occurrences<input type="number" min="1" step="1" value={count()} onInput={event => setCount(event.currentTarget.value)}/></label></Show><Show when={ending() === 'date'}><label>Last occurrence<input type="date" value={until()} onInput={event => setUntil(event.currentTarget.value)}/></label></Show></Show>
                    <For each={alerts()}>{(value, index) => <div class="calendar-alert-row"><label>Alert {index() + 1}<select value={value} onChange={event => { setAlertsChanged(true); setAlerts(previous => previous.map((value, i) => i === index() ? event.currentTarget.value : value)); }}><For each={[0, 5, 10, 15, 30, 60, 120, 1440, 2880]}>{minutes => <option value={String(minutes)}>{minutes ? `${minutes} minutes before` : 'At time of event'}</option>}</For><Show when={!['0','5','10','15','30','60','120','1440','2880','absolute'].includes(value)}><option value={value}>{value} minutes before</option></Show><option value="absolute">At date and time</option></select></label><Show when={value !== 'absolute'}><input aria-label={'Minutes before alert ' + (index() + 1)} type="number" value={value} onInput={event => { setAlertsChanged(true); setAlerts(previous => previous.map((value, i) => i === index() ? event.currentTarget.value : value)); }}/></Show><Show when={value === 'absolute'}><input aria-label={'Alert date ' + (index() + 1)} type="datetime-local" value={absolute()[index()]} onInput={event => { setAlertsChanged(true); setAbsolute(previous => previous.map((value, i) => i === index() ? event.currentTarget.value : value)); }}/></Show><button type="button" aria-label={'Remove alert ' + (index() + 1)} onClick={() => { setAlertsChanged(true); setAlerts(previous => previous.filter((_, i) => i !== index())); setAbsolute(previous => previous.filter((_, i) => i !== index())); }}>Remove</button></div>}</For>
                    <Show when={alerts().length < 2}><button type="button" onClick={() => { setAlertsChanged(true); setAlerts(previous => [...previous, '10']); setAbsolute(previous => [...previous, '']); }}>Add alert</button></Show>
                    <label>Location<input value={location()} onInput={event => setLocation(event.currentTarget.value)}/></label><label>URL<input type="url" value={url()} onInput={event => setURL(event.currentTarget.value)}/></label><label>Notes<textarea value={notes()} onInput={event => setNotes(event.currentTarget.value)}/></label>
                    <label>Show as<select value={availability()} onChange={event => setAvailability(event.currentTarget.value)}><option value="-1">Not supported</option><option value="0">Busy</option><option value="1">Free</option><option value="2">Tentative</option><option value="3">Unavailable</option></select></label>
                </Show>
            </Show>
            <Show when={editing && recurring}><fieldset><legend>Apply to repeating event</legend><label class="calendar-check"><input type="radio" name="span" checked={span() === 'this'} onChange={() => setSpan('this')}/>This Event Only</label><label class="calendar-check"><input type="radio" name="span" checked={span() === 'future'} onChange={() => setSpan('future')}/>All Future Events</label></fieldset></Show>
            <Show when={error()}><p role="alert">{error()}</p></Show><footer><button type="button" classList={{ destructive: props.edit.deleting }} onClick={save}>{props.edit.deleting ? 'Delete event' : props.edit.calendar ? 'Create calendar' : 'Save event'}</button></footer>
        </form>
    </dialog>;
}
