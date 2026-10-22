import { createEffect, createMemo, createSignal, For, Show, type Accessor } from 'solid-js';
import { auth } from '../auth/session';
import { router } from '../app/router';
import { withInput } from '../data/input';
import type { BridgeRecord } from '../data/types';
import { createCapabilities } from '../reminders/capabilities';
import { text, type ContactsPage } from './store';
import './groups.css';
type MembershipOperation = 'contacts.add_member' | 'contacts.remove_member';
type MembershipArgs = { id: string; contact_id: string };
type Change = { id: string; op: MembershipOperation; args: MembershipArgs };
const failed = new Set(['failed', 'rejected', 'rejected_by_owner', 'expired', 'delivery_failed']);
const members = (group: BridgeRecord) => Array.isArray(group.data.member_ids) ? group.data.member_ids.filter((id): id is string => typeof id === 'string') : [];
function useGroupCommands(page: Accessor<ContactsPage>) {
    const live = auth.live()!, capabilities = createCapabilities(live, 'contacts');
    const [pending, setPending] = createSignal<Change>(), [error, setError] = createSignal('');
    const state = createMemo(() => live.pending().find(command => command.id === pending()?.id));
    createEffect(() => {
        const change = pending(), command = state();
        if (!change || !command) return;
        if (failed.has(command.status)) { setError('Couldn’t save this list change. Try again.'); setPending(undefined); return; }
        const cursor = page()._cursor;
        if (command.status !== 'observed' || !command.commit_seq || !command.cursor || !cursor || cursor.epoch !== command.cursor.epoch || cursor.subscription_revision !== command.cursor.subscription_revision || cursor.sequence < command.commit_seq) return;
        const ids = command.observed_record_ids ?? [], groups = page().groups ?? [];
        if (!ids.length) return;
        const captured = groups.find(entry => entry.group.id === ids[0])?.group;
        if (!captured) return;
        if (captured && change.op === 'contacts.add_member' && !members(captured).includes(String(change.args.contact_id))) return;
        if (captured && change.op === 'contacts.remove_member' && members(captured).includes(String(change.args.contact_id))) return;
        setPending(undefined);
    });
    return {
        allowed: capabilities.allowed, pending, error, uncertain: () => state()?.status === 'indeterminate',
        issue(event: Event, op: MembershipOperation, args: MembershipArgs, before: BridgeRecord) {
            if (pending()) return false;
            setError('');
            try {
                if (!capabilities.allowed(op)) throw new Error('This list action is unavailable.');
                const id = withInput(event, () => live.command(op, args, undefined, before.version));
                setPending({ id, op, args }); return true;
            } catch { setError('Couldn’t save this list change. Try again.'); return false; }
        },
        check(event: Event) { const change = pending(); if (change) try { withInput(event, () => live.checkCommand(change.id)); } catch { setError('Couldn’t check this change. Try again.'); } },
    };
}
function Feedback(props: { commands: ReturnType<typeof useGroupCommands> }) {
    return <><Show when={props.commands.error()}><p class="contact-list-feedback" role="alert">{props.commands.error()}</p></Show><Show when={props.commands.pending()}><p class="contact-list-feedback" role="status"><Show when={props.commands.uncertain()} fallback="Saving…">Couldn’t confirm this change. <button class="button subtle" onClick={event => props.commands.check(event)}>Check status</button></Show></p></Show></>;
}
export function ContactLists(props: { page: ContactsPage; contact: BridgeRecord; current: string }) {
    const [target, setTarget] = createSignal('');
    const commands = useGroupCommands(() => props.page);
    const compatible = createMemo(() => props.contact.version > 0 && props.contact.data.writable === true ? (props.page.groups ?? []).filter(entry => entry.group.data.writable === true && entry.group.data.container_id === props.contact.data.container_id) : []);
    const joined = createMemo(() => compatible().filter(entry => members(entry.group).includes(props.contact.id)));
    const available = createMemo(() => compatible().filter(entry => !members(entry.group).includes(props.contact.id)));
    createEffect(() => { if (!available().some(entry => entry.group.id === target())) setTarget(''); });
    return <Show when={compatible().length && (commands.allowed('contacts.add_member') || commands.allowed('contacts.remove_member'))}><section class="contact-memberships" aria-label="Lists for this contact"><h2>Lists</h2>
        <ul><For each={joined()}>{entry => <li><span>{text(entry.group.data.name)}</span><Show when={commands.allowed('contacts.remove_member')}><button class="button subtle" disabled={Boolean(commands.pending())} aria-label={'Remove from ' + text(entry.group.data.name)} onClick={event => { if (commands.issue(event, 'contacts.remove_member', { id: entry.group.id, contact_id: props.contact.id }, entry.group) && props.current === entry.group.id) router.navigate('/contacts/list/' + encodeURIComponent(entry.group.id)); }}>Remove</button></Show></li>}</For></ul>
        <Show when={commands.allowed('contacts.add_member') && available().length}><div class="contact-membership-add"><select aria-label="Add to list" value={target()} disabled={Boolean(commands.pending())} onChange={event => setTarget(event.currentTarget.value)}><option value="">Choose a list</option><For each={available()}>{entry => <option value={entry.group.id}>{text(entry.group.data.name)}</option>}</For></select><button class="button subtle" disabled={!target() || Boolean(commands.pending())} onClick={event => { const group = available().find(entry => entry.group.id === target())?.group; if (group) commands.issue(event, 'contacts.add_member', { id: group.id, contact_id: props.contact.id }, group); }}>Add</button></div></Show><Feedback commands={commands}/>
    </section></Show>;
}
