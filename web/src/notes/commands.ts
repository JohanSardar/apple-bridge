import { createEffect, createRoot, createSignal, onCleanup } from 'solid-js';
import { auth } from '../auth/session';
import { withInput } from '../data/input';
import type { LiveClient } from '../data/live';
import type { BridgeRecord, CommandState } from '../data/types';
import { noteCapabilities } from './capabilities';
import { text } from './store';
export type Draft = { record: BridgeRecord; text: string; base: string; command?: string; sent?: string; error?: string; conflict?: boolean; lastSave: number };
function dirty(draft: Draft) { return draft.text !== draft.base || Boolean(draft.command); }
let retained: { live: LiveClient; drafts: Map<string, Draft> } | undefined;
createRoot(() => createEffect(() => {
    const live = auth.live(), session = auth.session();
    if (!session || retained && retained.live !== live) retained = undefined;
}));
window.addEventListener('beforeunload', event => {
    if (retained && [...retained.drafts.values()].some(dirty)) { event.preventDefault(); event.returnValue = ''; }
});
export function createNoteCommands() {
    const live = auth.live()!;
    const capabilities = noteCapabilities(live);
    const restored = retained?.live === live ? retained.drafts : new Map<string, Draft>();
    retained = undefined;
    const [drafts, setDrafts] = createSignal(new Map(restored)), [error, setError] = createSignal('');
    const [active, setActive] = createSignal<string>();
    const [creation, setCreation] = createSignal<{ id: string; op: string; title: string }>();
    const watchers = new Map<string, ReturnType<typeof live.subscribe>>();
    const update = (id: string, change: Partial<Draft>) => setDrafts(previous => { const next = new Map(previous), draft = next.get(id); if (draft) next.set(id, { ...draft, ...change }); return next; });
    const failed = (command: CommandState) => ['failed', 'rejected', 'rejected_by_owner', 'expired', 'delivery_failed'].includes(command.status);
    const drop = (id: string) => {
        watchers.get(id)?.dispose(); watchers.delete(id);
        setDrafts(previous => { const next = new Map(previous); next.delete(id); return next; });
    };
    const release = (id: string) => {
        if (active() === id) setActive();
        const draft = drafts().get(id);
        if (draft && !dirty(draft)) drop(id);
    };
    const watch = (id: string) => { if (!watchers.has(id)) watchers.set(id, live.subscribe('record', { collection: 'notes.note', id }, { cache: false, resetOnReconnect: true })); };
    for (const [id, draft] of restored) if (draft.command) watch(id);
    const begin = (record: BridgeRecord) => {
        watch(record.id);
        setActive(record.id);
        if (drafts().has(record.id)) return;
        setDrafts(previous => new Map(previous).set(record.id, { record, text: text(record.data.plain_text), base: text(record.data.plain_text), lastSave: -Infinity }));
    };
    createEffect(() => {
        const commands = new Map(live.pending().map(command => [command.id, command]));
        for (const [id, draft] of drafts()) {
            const command = draft.command ? commands.get(draft.command) : undefined;
            const watcher = watchers.get(id), observed = watcher?.data().records?.find(record => record.id === id);
            if (command && failed(command) && !draft.error) {
                const conflict = text((command.error as { code?: string } | undefined)?.code) === 'conflict';
                update(id, { error: conflict ? 'Changed elsewhere' : 'Couldn’t save. Your draft is kept.', conflict, command: undefined });
            } else if (command?.status === 'indeterminate' && !draft.error) {
                update(id, { error: 'Save status unknown. Your draft is kept.' });
            } else if (command && observed && command.observed_record_ids?.includes(id) && command.commit_seq && command.cursor && watcher?.cursor()?.epoch === command.cursor.epoch && watcher.cursor()!.subscription_revision === command.cursor.subscription_revision && watcher.cursor()!.sequence >= command.commit_seq && text(observed.data.plain_text) === draft.sent) {
                if (active() !== id && draft.text === draft.sent) drop(id);
                else update(id, { record: observed, base: draft.sent!, command: undefined, sent: undefined, error: undefined, conflict: false });
            } else if (!command && observed && observed.version > draft.record.version) {
                if (dirty(draft) && !draft.conflict) update(id, { conflict: true, error: 'Changed elsewhere' });
                else if (!dirty(draft)) update(id, { record: observed, base: text(observed.data.plain_text), text: text(observed.data.plain_text), error: undefined, conflict: false });
            }
        }
    });
    function issue(event: Event, op: string, args: Record<string, unknown>, version?: number) {
        setError('');
        try {
            if (!capabilities.allowed(op)) throw new Error(capabilities.proof(op).reason || 'This action is unavailable.');
            return withInput(event, () => live.command(op, args, undefined, version));
        } catch (failure) { setError(failure instanceof Error ? failure.message : 'Couldn’t save.'); return; }
    }
    const save = (event: Event, id: string, throttle = false) => {
        const draft = drafts().get(id);
        if (!draft || !dirty(draft)) return true;
        if (draft.command) return true;
        if (draft.conflict || draft.error) return false;
        if (throttle && performance.now() - draft.lastSave < 800) return true;
        const command = issue(event, 'notes.update', { id, plain_text: draft.text, mode: 'replace' }, draft.record.version);
        if (!command) return false;
        update(id, { command, sent: draft.text, lastSave: performance.now() });
        return true;
    };
    const saveAll = (event: Event) => { let okay = true; for (const [id, draft] of drafts()) if (dirty(draft) && !save(event, id)) okay = false; return okay; };
    const latest = (id: string) => { active(); return watchers.get(id)?.data().records?.find(value => value.id === id); };
    const reload = (id: string) => {
        const record = latest(id);
        if (record) update(id, { record, text: text(record.data.plain_text), base: text(record.data.plain_text), conflict: false, error: undefined, command: undefined, sent: undefined });
    };
    const create = (event: Event, op: string, args: Record<string, unknown>) => {
        if (creation()) return;
        const id = issue(event, op, args);
        if (id) setCreation({ id, op, title: text(args.title) });
        return id;
    };
    const creationState = () => live.pending().find(command => command.id === creation()?.id);
    const creationFailed = () => { const command = creationState(); return command && failed(command); };
    const unsaved = () => [...drafts().values()].some(dirty);
    const unload = (event: BeforeUnloadEvent) => { if (unsaved()) { event.preventDefault(); event.returnValue = ''; } };
    window.addEventListener('beforeunload', unload);
    onCleanup(() => {
        const kept = new Map([...drafts()].filter(([, draft]) => dirty(draft)));
        retained = auth.live() === live && auth.session() && kept.size ? { live, drafts: kept } : undefined;
        window.removeEventListener('beforeunload', unload); watchers.forEach(watcher => watcher.dispose());
    });
    return { drafts, error, capabilities, begin, release, save, saveAll, reload, canReload: (id: string) => Boolean(latest(id)), issue, create, creation, creationState, creationFailed, clearCreation: () => setCreation(), dirty, unsaved, edit: (id: string, value: string) => update(id, { text: value }) };
}
