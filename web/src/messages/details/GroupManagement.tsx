import { createEffect, createMemo, For, Show } from 'solid-js';
import type { LiveClient } from '../../data/live';
import type { BridgeRecord, Cursor } from '../../data/types';
import type { Handles } from '../store/types';
import { array } from '../store/types';
import { Avatar } from '../list/Avatar';
import { createGroupCommands, type GroupAction } from './groupCommands';

type Props = {
    live: LiveClient;
    record: BridgeRecord;
    handles: Handles;
    cursor?: Cursor;
    contact?: string;
    onContact: (id: string) => void;
};
const titles: Record<GroupAction, string> = {
    rename_group: 'Change Group Name',
    add_member: 'Add Member',
    remove_member: 'Remove Member',
    leave_group: 'Leave this Conversation',
};
const buttons: Record<GroupAction, string> = {
    rename_group: 'Save Name',
    add_member: 'Add Member',
    remove_member: 'Remove Member',
    leave_group: 'Leave Conversation',
};
export function GroupManagement(props: Props) {
    const commands = createGroupCommands(props.live, () => props.record, () => props.handles, () => props.cursor);
    const participants = () => array(props.record.data.participants).filter((id): id is string => typeof id === 'string');
    let field: HTMLInputElement | undefined;
    const editorIdentity = createMemo(() => commands.editor()?.action);
    createEffect(() => {
        const editor = editorIdentity();
        if (editor) queueMicrotask(() => { field?.focus(); field?.select(); });
    });
    return <section class="details-members" aria-label="Conversation members">
        <div class="details-group-heading">
            <h3>Members</h3>
            <Show when={commands.allowed('add_member')}>
                <button type="button" disabled={commands.busy()} onClick={() => commands.open('add_member')}>Add Member</button>
            </Show>
        </div>
        <For each={participants()}>{id => {
            const person = () => props.handles.get(id);
            return <div class="details-member">
                <Avatar title={person()?.display_name ?? ''} handle={id} handles={props.handles}/>
                <div><strong>{person()?.display_name || 'Unknown Sender'}</strong>
                    <Show when={person()?.address}><p class="secondary">{person()?.address}</p></Show>
                </div>
                <Show when={person()?.contact_id}>
                    <button type="button" aria-label={'Info for ' + (person()?.display_name || 'Unknown Sender')} aria-expanded={props.contact === id} onClick={() => props.onContact(id)}>Info</button>
                </Show>
                <Show when={commands.allowed('remove_member') && commands.address(id)}>
                    <button type="button" class="details-group-destructive" disabled={commands.busy()} aria-label={'Remove ' + (person()?.display_name || person()?.address)} onClick={() => commands.open('remove_member', commands.address(id))}>Remove</button>
                </Show>
            </div>;
        }}</For>
        <div class="details-group-actions">
            <Show when={commands.allowed('rename_group')}><button type="button" disabled={commands.busy()} onClick={() => commands.open('rename_group')}>Change Group Name</button></Show>
            <Show when={commands.allowed('leave_group')}><button type="button" class="details-group-destructive" disabled={commands.busy()} onClick={() => commands.open('leave_group')}>Leave this Conversation</button></Show>
        </div>
        <Show when={commands.editor()}>{editor => <form class="details-group-editor" aria-label={titles[editor().action]} onSubmit={event => event.preventDefault()} onKeyDown={event => { if (event.key === 'Enter' && event.target instanceof HTMLInputElement) { event.preventDefault(); commands.submit(event); } if (event.key === 'Escape' && !commands.busy()) { event.preventDefault(); event.stopPropagation(); commands.cancel(); } }}>
            <h4>{titles[editor().action]}</h4>
            <Show when={editor().action === 'rename_group'}>
                <label>Group name<input ref={field} value={editor().name} disabled={commands.busy()} maxlength={65536} autocomplete="off" onInput={event => commands.update('name', event.currentTarget.value)}/></label>
                <p class="secondary">Everyone in the conversation will see this name.</p>
            </Show>
            <Show when={editor().action === 'add_member'}>
                <label>Email or phone number<input ref={field} value={editor().handle} disabled={commands.busy()} autocomplete="off" spellcheck={false} onInput={event => commands.update('handle', event.currentTarget.value)}/></label>
                <p class="secondary">This person will be added to the conversation.</p>
            </Show>
            <Show when={editor().action === 'remove_member'}><p>Remove {editor().handle} from this conversation? This changes the group for everyone.</p></Show>
            <Show when={editor().action === 'leave_group'}><p>You will stop receiving messages in this conversation.</p></Show>
            <Show when={!commands.busy() && commands.inputProblem()}><p class="secondary">{commands.inputProblem()}</p></Show>
            <Show when={commands.message()}><p role="status" aria-live="polite">{commands.message()}</p></Show>
            <div class="details-group-editor-actions">
                <button type="button" disabled={commands.busy()} onClick={commands.cancel}>Cancel</button>
                <Show when={commands.canCheck()}><button type="button" onClick={commands.check}>Check Change</button></Show>
                <button type="button" onClick={commands.submit} classList={{ 'details-group-destructive': editor().action === 'remove_member' || editor().action === 'leave_group' }} disabled={!commands.canSubmit()}>{buttons[editor().action]}</button>
            </div>
        </form>}</Show>
    </section>;
}
