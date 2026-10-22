import { createSignal, onCleanup } from 'solid-js';
import { auth } from '../auth/session';
import { withInput } from '../data/input';
import type { BridgeRecord, CommandState, Cursor } from '../data/types';
import { createCapabilities } from '../reminders/capabilities';
import { object, text, type CalendarEntry, type CalendarPage } from './store';
export function nativeRepeat(command: CommandState) {
    const rules = object(command.evidence?.native_result).recurrence;
    return Array.isArray(rules) ? rules.map(object).map(rule => `Every ${Number(rule.interval) || 1} ${text(rule.frequency)}${Number(rule.count) > 0 ? ', ' + Number(rule.count) + ' times' : text(rule.end) ? ', until ' + text(rule.end) : ''}`).join('; ') || 'Does not repeat' : '';
}
type Change = { id: string; op: string; args: Record<string, unknown>; before?: BridgeRecord; baseline: Set<string>; expires: number };
const failed = new Set(['failed', 'rejected', 'rejected_by_owner', 'expired', 'delivery_failed']);
function committed(cursor: Cursor | undefined, command: CommandState) { return Boolean(cursor && command.cursor && command.commit_seq && cursor.epoch === command.cursor.epoch && cursor.subscription_revision === command.cursor.subscription_revision && cursor.sequence >= command.commit_seq); }
export function calendarCommands() {
    const live = auth.live()!, capabilities = createCapabilities(live, 'calendar');
    const [changes, setChanges] = createSignal<Change[]>([]), [error, setError] = createSignal(''), [tick, setTick] = createSignal(0);
    const timers = new Set<ReturnType<typeof setTimeout>>(), settling = new Set<string>();
    const later = (action: () => void, delay: number) => { const timer = setTimeout(() => { timers.delete(timer); action(); }, delay); timers.add(timer); };
    onCleanup(() => { for (const timer of timers) clearTimeout(timer); });
    const state = (id: string) => live.pending().find(command => command.id === id);
    const issue = (event: Event, op: string, args: Record<string, unknown>, page: CalendarPage, before?: BridgeRecord) => {
        setError('');
        try {
            if (!capabilities.allowed(op)) throw new Error(capabilities.proof(op).reason);
            if (changes().some(change => change.before?.id === before?.id && before && !failed.has(state(change.id)?.status ?? '') && !['observed', 'indeterminate'].includes(state(change.id)?.status ?? ''))) throw new Error('Wait for the previous change to finish.');
            const id = withInput(event, () => live.command(op, args, undefined, before?.version));
            if (op === 'calendar.create_event') later(() => setTick(value => value + 1), 1000);
            setChanges(previous => [...previous.slice(-63), { id, op, args, before, baseline: new Set(page.events?.map(entry => entry.event.id)), expires: performance.now() + 1000 }]);
            return id;
        } catch (failure) { setError(failure instanceof Error ? failure.message : 'Unable to save.'); }
    };
    const project = (input: CalendarPage): CalendarEntry[] => {
        tick();
        for (const change of changes()) {
            const command = state(change.id);
            if (command?.status === 'observed' && committed(input._cursor, command) && !settling.has(change.id)) {
                settling.add(change.id); later(() => { setChanges(previous => previous.filter(value => value.id !== change.id)); settling.delete(change.id); }, 0);
            }
        }
        const active = changes().filter(change => { const command = state(change.id); return command && !failed.has(command.status) && command.status !== 'indeterminate' && !committed(input._cursor as Cursor | undefined, command) && change.op !== 'calendar.create_calendar'; });
        const held = active.filter(change => change.op === 'calendar.create_event' && !state(change.id)?.observed_record_ids?.length && performance.now() < change.expires);
        let events = (input.events ?? []).filter(entry => !held.some(change => !change.baseline.has(entry.event.id)));
        for (const change of active) {
            const command = state(change.id)!, ids = command.observed_record_ids ?? [], oldID = change.before?.id;
            if (change.op === 'calendar.update_event' && change.args.span === 'future' && !ids.length) continue;
            if (change.op === 'calendar.delete_event') { events = events.filter(entry => entry.event.id !== oldID); continue; }
            if (!oldID && !ids.length && performance.now() >= change.expires) continue;
            const id = ids[0] ?? oldID ?? 'pending:' + change.id;
            const previous = events.find(entry => entry.event.id === id || entry.event.id === oldID);
            events = events.filter(entry => entry.event.id !== id && entry.event.id !== oldID);
            events.push({ ...(previous ?? { column: 0, columns: 1, overlap_group: 'pending' }), event: { ...(previous?.event ?? change.before ?? { source: 'calendar', collection: 'calendar.event', version: 0 }), id, data: { ...(previous?.event.data ?? change.before?.data ?? {}), ...change.args, _pending: true } } });
        }
        return events;
    };
    const messages = () => { tick(); return changes().flatMap(change => { const command = state(change.id); if (!command) return []; if (failed.has(command.status)) return [{ id: change.id, text: 'Couldn’t save. The Calendar view has been restored.', command }]; if (command.status === 'indeterminate') return [{ id: change.id, text: command.evidence?.native_result ? Array.isArray(change.args.recurrence) && (change.args.recurrence.length > 0 || (Array.isArray(object(command.evidence.native_result).recurrence) && (object(command.evidence.native_result).recurrence as unknown[]).length > 0)) ? 'Saved, but the repeat may differ' : 'Saved, but some event details may differ. Review the event before trying again.' : 'The result is uncertain. Check before trying again.', command }]; if (command.status !== 'observed') return [{ id: change.id, text: 'Saving…', command }]; return []; }); };
    return { issue, project, messages, error, capabilities, check: (id: string) => live.checkCommand(id) };
}
