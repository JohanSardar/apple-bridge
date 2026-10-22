import { createMemo, createSignal, For, Show, type Accessor } from 'solid-js';
import type { Item, List, Row } from './store';
import type { CommandState } from '../data/types';
import { text } from './store';
type BatchAction = 'complete' | 'uncomplete' | 'delete' | 'move';
type Issue = (event: Event, operation: string, args: Record<string, unknown>, before?: Item) => string | undefined;
const selectionLimit = 100;
export function createReminderSelection(rows: Accessor<Row[]>, allowed: (operation: string, item?: Item) => boolean, issue: Issue, states: Accessor<readonly CommandState[]>, availableRows: Accessor<Row[]> = rows) {
    const [enabled, setEnabled] = createSignal(false);
    const [selected, setSelected] = createSignal(new Map<string, Item>());
    const [message, setMessage] = createSignal('');
    const [confirmation, setConfirmation] = createSignal<{ action: BatchAction; items: Item[] }>();
    const [destination, setDestination] = createSignal('');
    const [receipt, setReceipt] = createSignal<{ id: string; title: string }[]>([]);
    const outcomes = createMemo(() => {
        const commands = new Map(states().map(command => [command.id, command]));
        return receipt().map(entry => {
            const command = commands.get(entry.id);
            const status = command?.status;
            const result = status === 'observed' ? 'Saved' : status === 'indeterminate' || command?.transport_lost ? 'Check status' : ['failed', 'rejected', 'rejected_by_owner', 'expired', 'delivery_failed'].includes(status ?? '') ? 'Not saved' : 'Saving';
            return { ...entry, result };
        });
    });
    let anchor: string | undefined;
    const selectable = (item: Item) => item.reminder.version > 0 && !item.reminder.data._pending && allowed('reminders.update', item);
    const choices = createMemo(() => availableRows().flatMap(row => row.item && selectable(row.item) ? [row.item] : []));
    const reset = () => {
        setSelected(new Map());
        setEnabled(false);
        setConfirmation(undefined);
        setDestination('');
        setMessage('');
        anchor = undefined;
    };
    function toggle(item: Item, extend = false) {
        if (!selectable(item)) return;
        setEnabled(true);
        setMessage('');
        const current = new Map(selected()), values = rows().flatMap(row => row.item && selectable(row.item) ? [row.item] : []);
        const start = values.findIndex(value => value.reminder.id === anchor);
        const end = values.findIndex(value => value.reminder.id === item.reminder.id);
        if (extend && start >= 0 && end >= 0) {
            for (const value of values.slice(Math.min(start, end), Math.max(start, end) + 1)) current.set(value.reminder.id, current.get(value.reminder.id) ?? value);
        } else {
            if (current.has(item.reminder.id)) current.delete(item.reminder.id);
            else current.set(item.reminder.id, item);
            anchor = item.reminder.id;
        }
        if (current.size > selectionLimit) { setMessage('Select up to 100 reminders at a time.'); return; }
        setSelected(current);
    }
    function selectAll() {
        const values = choices();
        if (values.length > selectionLimit) { setMessage('Select up to 100 reminders at a time.'); return; }
        setEnabled(true);
        setSelected(new Map(values.map(item => [item.reminder.id, item])));
        anchor = values[0]?.reminder.id;
    }
    const operation = (action: BatchAction) => action === 'move' ? 'reminders.update' : 'reminders.' + action;
    function relationshipError(values = [...selected().values()]) {
        const chosen = new Set(values.map(item => item.reminder.id));
        const current = new Map(availableRows().flatMap(row => row.item ? [[row.item.reminder.id, row.item] as const] : []));
        const pending = values.flatMap(item => (current.get(item.reminder.id) ?? item).subtasks), seen = new Set<string>();
        while (pending.length) {
            const item = pending.pop()!, id = item.reminder.id;
            if (chosen.has(id)) return 'Select parents and their subtasks separately.';
            if (seen.has(id)) continue;
            seen.add(id); pending.push(...item.subtasks);
        }
        return '';
    }
    function canRun(action: BatchAction, values = [...selected().values()]) {
        return values.length > 0 && values.length <= selectionLimit && !relationshipError(values) && values.every(item => !item.reminder.data._pending && allowed(operation(action), item));
    }
    function request(action: BatchAction) {
        const values = [...selected().values()];
        if (!canRun(action, values)) { setMessage('These reminders are no longer available for this action.'); return; }
        setDestination('');
        setConfirmation({ action, items: values });
    }
    function clearCompleted() {
        const values = choices().filter(item => item.reminder.data.completed === true);
        if (!canRun('delete', values)) { setMessage(relationshipError(values) || (values.length > selectionLimit ? 'Clear up to 100 completed reminders at a time.' : 'No completed reminders are available to clear.')); return; }
        setConfirmation({ action: 'delete', items: values });
    }
    function confirm(event: Event, lists: List[]) {
        if (!event.isTrusted) return;
        const pending = confirmation();
        if (!pending || !canRun(pending.action, pending.items)) return;
        const target = destination();
        if (pending.action === 'move' && !lists.some(entry => entry.list.id === target && entry.list.data.writable === true && entry.list.data.is_group !== true && entry.status !== 'unsupported')) return;
        const submitted: { id: string; title: string }[] = [];
        for (const item of pending.items) {
            const args: Record<string, unknown> = { id: item.reminder.id };
            if (pending.action === 'move') args.list_id = target;
            const id = issue(event, operation(pending.action), args, item);
            if (id) submitted.push({ id, title: text(item.reminder.data.title) });
        }
        setReceipt(submitted);
        setConfirmation(undefined);
        setSelected(new Map());
        setEnabled(false);
        setMessage(`${submitted.length} reminder${submitted.length === 1 ? '' : 's'} submitted. Each change is saved independently.`);
    }
    function intercept(event: MouseEvent, item: Item) {
        const titleClick = enabled() && event.target instanceof Element && Boolean(event.target.closest('.reminder-title-button,.reminder-notes'));
        if (!event.isTrusted || !(titleClick || event.metaKey || event.ctrlKey || event.shiftKey)) return;
        event.preventDefault();
        event.stopPropagation();
        toggle(item, event.shiftKey);
    }
    function keyboard(event: KeyboardEvent, item?: Item) {
        if (!event.isTrusted || event.isComposing) return false;
        if (event.key === 'Escape' && enabled()) { reset(); event.preventDefault(); return true; }
        if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'a') {
            event.preventDefault(); selectAll(); return true;
        }
        if (enabled() && event.shiftKey && ['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key) && item) {
            event.preventDefault();
            toggle(item, true);
            return true;
        }
        if ((event.metaKey || event.ctrlKey) && event.key === 'Enter' && selected().size) {
            event.preventDefault(); request([...selected().values()].every(item => item.reminder.data.completed === true) ? 'uncomplete' : 'complete'); return true;
        }
        if (event.key === ' ' && (enabled() || event.shiftKey) && item) {
            event.preventDefault(); toggle(item, event.shiftKey); return true;
        }
        if ((event.key === 'Delete' || event.key === 'Backspace') && selected().size) {
            event.preventDefault(); request('delete'); return true;
        }
        return false;
    }
    return { enabled, setEnabled, selected, message, outcomes, dismissReceipt: () => setReceipt([]), choices, relationshipError, confirmation, destination, setDestination, reset, toggle, selectAll, request, canRun, clearCompleted, confirm, intercept, keyboard, cancel: () => setConfirmation(undefined) };
}
export function SelectionToolbar(props: { selection: ReturnType<typeof createReminderSelection>; lists: List[]; showCompleted: boolean; check: (id: string) => void }) {
    const state = props.selection;
    const actionTitle = () => {
        const pending = state.confirmation();
        if (!pending) return '';
        return ({ complete: 'Complete', uncomplete: 'Reopen', move: 'Move', delete: 'Delete' })[pending.action];
    };
    return <>
        <div class="reminder-selection-toolbar" role="toolbar" aria-label="Reminder selection">
            <button class="button" aria-pressed={state.enabled()} onClick={() => state.enabled() ? state.reset() : state.setEnabled(true)}>{state.enabled() ? 'Done Selecting' : 'Select Reminders'}</button>
            <Show when={state.enabled()}>
                <span role="status">{state.selected().size} selected</span>
                <button class="button" onClick={state.selectAll}>Select All</button>
                <button class="button" disabled={!state.canRun('complete')} onClick={() => state.request('complete')}>Complete Selected</button>
                <button class="button" disabled={!state.canRun('uncomplete')} onClick={() => state.request('uncomplete')}>Reopen Selected</button>
                <button class="button" disabled={!state.canRun('move')} onClick={() => state.request('move')}>Move Selected</button>
                <button class="button danger" disabled={!state.canRun('delete')} onClick={() => state.request('delete')}>Delete Selected</button>
            </Show>
            <Show when={props.showCompleted && state.choices().some(item => item.reminder.data.completed === true)}><button class="button danger" onClick={state.clearCompleted}>Clear Completed in This View</button></Show>
        </div>
        <Show when={state.relationshipError()}><p class="reminder-selection-message" role="status">{state.relationshipError()}</p></Show>
        <Show when={state.message()}><p class="reminder-selection-message" role="status">{state.message()}</p></Show>
        <Show when={state.outcomes().length}><details class="reminder-selection-receipt"><summary>Last selection action</summary><ul><For each={state.outcomes()}>{entry => <li><span>{entry.title}</span><span>{entry.result}</span><Show when={entry.result === 'Check status'}><button class="button" onClick={() => props.check(entry.id)}>Check Status</button></Show></li>}</For></ul><button class="button" onClick={state.dismissReceipt}>Dismiss Results</button></details></Show>
        <Show when={state.confirmation()}>{pending => <dialog class="reminder-details" aria-label="Confirm Selection Action" ref={element => queueMicrotask(() => element.showModal())} onClose={state.cancel}>
            <h2>{actionTitle()} {pending().items.length} reminder{pending().items.length === 1 ? '' : 's'}?</h2>
            <p>The selected reminders are listed below. Deleting a parent includes its subtasks. Each change is saved independently; failures are restored without retry.</p>
            <ul class="reminder-selection-preview"><For each={pending().items}>{item => <li>{text(item.reminder.data.title)}</li>}</For></ul>
            <Show when={pending().action === 'move'}><label class="reminder-move-label">Destination list<select aria-label="Destination list" value={state.destination()} onChange={event => state.setDestination(event.currentTarget.value)}><option value="">Choose a list</option><For each={props.lists.filter(entry => entry.list.data.writable === true && entry.list.data.is_group !== true && entry.status !== 'unsupported')}>{entry => <option value={entry.list.id}>{text(entry.list.data.title)}</option>}</For></select></label></Show>
            <footer><button class="button" autofocus onClick={state.cancel}>Cancel</button><button class="button primary" disabled={pending().action === 'move' && !state.destination()} onClick={event => state.confirm(event, props.lists)}>{actionTitle()} Reminders</button></footer>
        </dialog>}</Show>
    </>;
}
