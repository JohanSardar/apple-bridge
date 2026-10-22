import { createMemo, createSignal, For, Show } from 'solid-js';
import type { Item, List } from './store';
import { text } from './store';
import { dueFields, parseReminder } from './dates';
export function ReminderEditor(props: { item?: Item; listID: string; lists: List[]; save: (event: Event, args: Record<string, unknown>) => boolean; cancel: () => void }) {
    const data = props.item?.reminder.data ?? {}, initial = text(data.due), instant = initial && !/^\d{4}-\d{2}-\d{2}$/u.test(initial) ? new Date(initial) : undefined;
    const [title, setTitle] = createSignal(text(data.title)), [notes, setNotes] = createSignal(text(data.notes)), [url, setURL] = createSignal(text(data.url)), [list, setList] = createSignal(props.listID), [priority, setPriority] = createSignal(Number(data.priority) || 0), [error, setError] = createSignal('');
    const [date, setDate] = createSignal<string | undefined>(props.item ? instant ? [instant.getFullYear(), String(instant.getMonth() + 1).padStart(2, '0'), String(instant.getDate()).padStart(2, '0')].join('-') : initial : undefined);
    const [time, setTime] = createSignal<string | undefined>(props.item ? instant ? `${String(instant.getHours()).padStart(2, '0')}:${String(instant.getMinutes()).padStart(2, '0')}` : '' : undefined);
    const initialDate = date() ?? '', initialTime = time() ?? '', initialPriority = priority();
    const parsed = createMemo(() => props.item ? { title: title().trim(), date: date() ?? '', time: time() ?? '' } : parseReminder(title()));
    const save = (event: Event) => {
        if (!event.isTrusted || !(event.currentTarget instanceof HTMLElement) || !event.currentTarget.closest('form')?.reportValidity()) return;
        setError('');
        try {
            const value = parsed(), selectedDate = date() ?? value.date, selectedTime = time() ?? value.time;
            const result: Record<string, unknown> = {};
            if (!props.item || value.title !== text(data.title).trim()) result.title = value.title;
            if (!props.item || notes() !== text(data.notes)) result.notes = notes();
            if (!props.item || url() !== text(data.url)) result.url = url();
            if (!props.item || list() !== props.listID) result.list_id = list();
            if (!props.item || priority() !== initialPriority) result.priority = priority();
            if (!props.item || selectedDate !== initialDate || selectedTime !== initialTime) Object.assign(result, dueFields(selectedDate, selectedTime));
            if (props.item && !Object.keys(result).length) { props.cancel(); return; }
            if (!value.title || !list()) return;
            props.save(event, result);
        } catch (failure) { setError(failure instanceof Error ? failure.message : 'Choose a valid reminder date.'); }
    };
    return <form class="reminder-editor" aria-label={props.item ? 'Edit Reminder' : 'New Reminder'} onSubmit={event => event.preventDefault()}>
        <label>Title<input ref={element => queueMicrotask(() => { if (element.isConnected) element.focus(); })} autofocus required aria-label="Reminder title" value={title()} placeholder="New Reminder" onInput={event => setTitle(event.currentTarget.value)} onKeyDown={event => { if (event.key === 'Enter' && !event.isComposing) { event.preventDefault(); save(event); } if (event.key === 'Escape') props.cancel(); }}/></label>
        <Show when={!props.item && parsed().date && date() === undefined}><p class="reminder-date-preview">{parsed().title} · {parsed().date}{parsed().time ? ' ' + parsed().time : ''}</p></Show>
        <label>Notes<textarea aria-label="Reminder notes" value={notes()} onInput={event => setNotes(event.currentTarget.value)}/></label>
        <label>URL<input type="url" aria-label="Reminder URL" value={url()} onInput={event => setURL(event.currentTarget.value)}/></label>
        <div class="reminder-editor-dates"><label>Date<input type="date" aria-label="Reminder date" value={date() ?? parsed().date} onInput={event => { setDate(event.currentTarget.value); if (!event.currentTarget.value) setTime(''); }}/></label><label>Time<input type="time" aria-label="Reminder time" disabled={!(date() ?? parsed().date)} value={time() ?? parsed().time} onInput={event => setTime(event.currentTarget.value)}/></label></div>
        <label>List<select aria-label="Reminder list" value={list()} onChange={event => setList(event.currentTarget.value)}><For each={props.lists.filter(entry => entry.list.data.writable === true && entry.list.data.is_group !== true && entry.status !== 'unsupported')}>{entry => <option value={entry.list.id}>{text(entry.list.data.title)}</option>}</For></select></label>
        <label>Priority<select aria-label="Reminder priority" value={priority()} onChange={event => setPriority(Number(event.currentTarget.value))}><option value="0">None</option><option value="9">Low</option><option value="5">Medium</option><option value="1">High</option></select></label>
        <Show when={error()}><p role="alert">{error()}</p></Show>
        <footer><button type="button" class="button" onClick={props.cancel}>Cancel</button><button type="button" class="button primary" disabled={!parsed().title || !list()} onClick={save}>{props.item ? 'Save Reminder' : 'Add Reminder'}</button></footer>
    </form>;
}
export function NewList(props: { save: (event: Event, title: string) => void; cancel: () => void }) {
    const [title, setTitle] = createSignal('');
    return <dialog class="reminder-details reminder-new-list" aria-label="New List" ref={element => queueMicrotask(() => element.showModal())} onClose={props.cancel}><h2>New List</h2><label>List name<input autofocus aria-label="List name" value={title()} onInput={event => setTitle(event.currentTarget.value)} onKeyDown={event => { if (event.key === 'Enter' && !event.isComposing && title().trim()) { event.preventDefault(); props.save(event, title().trim()); } }}/></label><footer><button class="button" onClick={props.cancel}>Cancel</button><button class="button primary" disabled={!title().trim()} onClick={event => props.save(event, title().trim())}>Create List</button></footer></dialog>;
}
