import { createEffect, createSignal, onCleanup, type Accessor } from 'solid-js';
import type { LiveClient } from '../data/live';
import type { BridgeRecord, CommandState, Cursor } from '../data/types';
import { withInput } from '../data/input';
import { createCapabilities } from './capabilities';
import type { Item, Items, Lists } from './store';
type Change = { id: string; op: string; args: Record<string, unknown>; before?: Item; watch?: ReturnType<LiveClient['subscribe']>; expires?: number; folding?: boolean; settling?: boolean; baseline?: Set<string>; identityExpires?: number; slot?: { section: string; index: number; parent?: string } };
const failed = new Set(['failed', 'rejected', 'rejected_by_owner', 'expired', 'delivery_failed']);
function committed(cursor: Cursor | undefined, command: CommandState) { return Boolean(command.commit_seq && cursor && command.cursor && cursor.epoch === command.cursor.epoch && cursor.subscription_revision === command.cursor.subscription_revision && cursor.sequence >= command.commit_seq); }
export function createReminderCommands(live: LiveClient) {
    const capabilities = createCapabilities(live, 'reminders'), changes = new Map<string, Change>(), timers = new Set<ReturnType<typeof setTimeout>>();
    const [revision, setRevision] = createSignal(0), [error, setError] = createSignal('');
    const changed = () => setRevision(value => value + 1);
    let lastItems: Items = {}, lastLists: Lists = {};
    const states = () => new Map(live.pending().map(command => [command.id, command]));
    const later = (action: () => void, delay: number) => { const timer = setTimeout(() => { timers.delete(timer); action(); changed(); }, delay); timers.add(timer); };
    createEffect(() => { for (const command of live.pending()) if (changes.has(command.id) && failed.has(command.status)) setError('Couldn’t save this change. Your reminders were restored.'); });
    function issue(event: Event, op: string, args: Record<string, unknown>, before?: Item) {
        setError('');
        try {
            if (!capabilities.allowed(op)) throw new Error(capabilities.proof(op).reason || 'This action is unavailable.');
            const id = withInput(event, () => live.command(op, args, undefined, before?.reminder.version));
            const change: Change = { id, op, args, before };
            if (op === 'reminders.create' || op === 'reminders.create_list') {
                const ids = new Set<string>();
                const collect = (items: Item[]) => { for (const item of items) { ids.add(item.reminder.id); collect(item.subtasks); } };
                if (op === 'reminders.create') for (const group of lastItems.sections ?? []) collect(group.items);
                else for (const entry of lastLists.lists ?? []) ids.add(entry.list.id);
                change.baseline = ids; change.identityExpires = performance.now() + 1000; later(() => {}, 1000);
            }
            if (before) {
                const locate = (items: Item[], section: string, parent?: string) => { items.forEach((item, index) => { if (item.reminder.id === before.reminder.id) change.slot = { section, index, parent }; locate(item.subtasks, section, item.reminder.id); }); };
                for (const group of lastItems.sections ?? []) locate(group.items, group.section.id);
            }
            if (before) change.watch = live.subscribe('record', { collection: 'reminders.reminder', id: before.reminder.id }, { cache: false, resetOnReconnect: true });
            if (op === 'reminders.complete') { change.expires = performance.now() + 1550; later(() => { change.folding = true; }, 900); later(() => {}, 1550); }
            changes.set(id, change); changed();
            return id;
        } catch (failure) { setError(failure instanceof Error ? failure.message : 'Unable to save.'); return; }
    }
    const project = (source: Accessor<Items>, selector: Accessor<string>, showCompleted: Accessor<boolean>) => () => {
        revision(); const input = source(), commands = states(); lastItems = input;
        const active = [...changes.values()].filter(change => { const command = commands.get(change.id); return command && !failed.has(command.status) && change.op !== 'reminders.create_list'; });
        if (!active.length || input.error) return input;
        const byID = new Map<string, Item>(), grouped = (input.sections ?? []).map(group => ({ ...group, items: group.items }));
        const collect = (items: Item[]) => { for (const item of items) { byID.set(item.reminder.id, item); collect(item.subtasks); } }; grouped.forEach(group => collect(group.items));
        const patches = new Map<string, Item | null>(), append: { item: Item; slot?: Change['slot'] }[] = [];
        const waiting = active.filter(change => change.op === 'reminders.create' && selector() === change.args.list_id && !commands.get(change.id)?.observed_record_ids?.length && performance.now() < (change.identityExpires ?? 0));
        const held = new Set([...byID.keys()].filter(id => waiting.some(change => !change.baseline?.has(id))));
        const retainedChildren = new Set<string>();
        const latest = new Map(active.filter(change => change.before).map(change => [change.before!.reminder.id, change.id]));
        for (const change of active) {
            const command = commands.get(change.id)!, ids = command.observed_record_ids ?? [], existingID = change.before?.reminder.id;
            if (existingID && latest.get(existingID) !== change.id) continue;
            const observed = change.watch?.data().records?.find(record => ids.includes(record.id));
            const linger = change.op === 'reminders.complete' && !showCompleted() && selector() !== 'smart:completed' && performance.now() < (change.expires ?? 0);
            if (committed(input._cursor, command) && !linger) {
                if (!change.settling) { change.settling = true; later(() => { change.watch?.dispose(); changes.delete(change.id); }, 0); }
                continue;
            }
            if (change.op === 'reminders.delete') { if (existingID) patches.set(existingID, null); continue; }
            const base = observed ?? byID.get(ids[0] ?? existingID ?? '')?.reminder ?? change.before?.reminder;
            const id = ids[0] ?? existingID ?? 'pending:' + change.id;
            if (!existingID && !ids.length && performance.now() >= (change.identityExpires ?? 0)) continue;
            const data: Record<string, unknown> = { ...(base?.data ?? {}), ...change.args, ...(change.op === 'reminders.complete' ? { completed: true } : change.op === 'reminders.uncomplete' ? { completed: false } : {}), _creation: change.op === 'reminders.create' ? change.id : undefined, _pending: command.status !== 'observed' || !ids.length || !committed(observed ? change.watch?.cursor() : input._cursor, command), _completing: linger, _folding: linger && change.folding };
            delete data.id;
            const reminder: BridgeRecord = { id, collection: 'reminders.reminder', source: 'reminders', version: base?.version ?? 0, data };
            const item: Item = { ...(byID.get(id) ?? change.before ?? { subtasks: [], attachments: [] }), reminder };
            if (linger && !byID.has(id)) {
                const retain = (children: Item[]): Item[] => children.map(child => { retainedChildren.add(child.reminder.id); const current = byID.get(child.reminder.id) ?? child; return { ...current, subtasks: retain(child.subtasks) }; });
                item.subtasks = retain(change.before?.subtasks ?? []);
            }
            const belongs = selector().startsWith('smart:') || selector().startsWith('tag:') || data.list_id === selector();
            if (existingID && id !== existingID) patches.set(existingID, null);
            const visible = belongs && (showCompleted() || selector() === 'smart:completed' || !data.completed || linger);
            if (existingID || byID.has(id)) patches.set(id, visible ? item : null);
            if (!byID.has(id) && visible) append.push({ item, slot: change.slot });
        }
        const visit = (items: Item[]): Item[] => items.flatMap(item => { if (held.has(item.reminder.id) || retainedChildren.has(item.reminder.id)) return []; const selected = patches.has(item.reminder.id) ? patches.get(item.reminder.id) : item; return selected ? [{ ...selected, subtasks: visit(selected.subtasks) }] : []; });
        const sections = grouped.map(group => ({ ...group, items: visit(group.items) }));
        for (const { item, slot } of new Map(append.map(value => [value.item.reminder.id, value])).values()) {
            let section = sections.find(group => group.section.id === slot?.section) ?? sections.find(group => group.section.kind === 'none');
            if (!section) { section = { section: { id: 'pending', kind: 'none', title: '' }, items: [] }; sections.push(section); }
            let siblings = section.items;
            if (slot?.parent) {
                const findParent = (children: Item[]): Item | undefined => { for (const child of children) { if (child.reminder.id === slot.parent) return child; const parent = findParent(child.subtasks); if (parent) return parent; } };
                const parent = findParent(siblings); if (parent) siblings = parent.subtasks;
            }
            siblings.splice(Math.min(slot?.index ?? siblings.length, siblings.length), 0, item);
        }
        return { ...input, sections };
    };
    const projectLists = (source: Accessor<Lists>) => () => {
        revision(); const input = source(), commands = states(); lastLists = input;
        for (const change of changes.values()) {
            const command = commands.get(change.id);
            if (change.op === 'reminders.create_list' && command && committed(input._cursor, command) && command.observed_record_ids?.some(id => input.lists?.some(entry => entry.list.id === id)) && !change.settling) {
                change.settling = true; later(() => changes.delete(change.id), 0);
            }
        }
        const waiting = [...changes.values()].filter(change => change.op === 'reminders.create_list' && !failed.has(commands.get(change.id)?.status ?? '') && !commands.get(change.id)?.observed_record_ids?.length && performance.now() < (change.identityExpires ?? 0));
        return waiting.length ? { ...input, lists: input.lists?.filter(entry => !waiting.some(change => !change.baseline?.has(entry.list.id))) } : input;
    };
    const pendingLists = (lists: Lists) => { revision(); const commands = states(); return [...changes.values()].filter(change => { const command = commands.get(change.id); return change.op === 'reminders.create_list' && command && !failed.has(command.status) && !committed(lists._cursor, command) && (!command.observed_record_ids?.length ? performance.now() < (change.identityExpires ?? 0) : !lists.lists?.some(entry => command.observed_record_ids?.includes(entry.list.id))); }).map(change => ({ id: change.id, title: String(change.args.title) })); };
    const pending = () => { revision(); return [...changes.values()].some(change => { const command = states().get(change.id); return command && !failed.has(command.status) && command.status !== 'observed' && command.status !== 'indeterminate'; }); };
    const uncertain = () => { revision(); return [...changes.keys()].filter(id => states().get(id)?.status === 'indeterminate' || changes.get(id)?.identityExpires !== undefined && performance.now() >= changes.get(id)!.identityExpires! && !states().get(id)?.observed_record_ids?.length && !failed.has(states().get(id)?.status ?? '')); };
    onCleanup(() => { for (const timer of timers) clearTimeout(timer); for (const change of changes.values()) change.watch?.dispose(); capabilities.dispose(); });
    return { issue, project, projectLists, pendingLists, pending, uncertain, error, capabilities, check: (id: string) => live.checkCommand(id) };
}
