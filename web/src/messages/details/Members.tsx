import { For, Show } from 'solid-js';
import type { BridgeRecord } from '../../data/types';
import { array, type Handles } from '../store/types';
import { Avatar } from '../list/Avatar';
export function ConversationMembers(props: { record: BridgeRecord; handles: Handles; contact?: string; onContact: (id: string) => void }) {
    const participants = () => array(props.record.data.participants).filter((id): id is string => typeof id === 'string');
    return <section class="details-members" aria-label="Conversation members"><h3>Members</h3>
        <For each={participants()}>{id => {
            const person = () => props.handles.get(id);
            return <div class="details-member"><Avatar title={person()?.display_name ?? ''} handle={id} handles={props.handles}/>
                <div><strong>{person()?.display_name || 'Unknown Sender'}</strong><Show when={person()?.address}><p class="secondary">{person()?.address}</p></Show></div>
                <Show when={person()?.contact_id}><button type="button" aria-label={'Info for ' + (person()?.display_name || 'Unknown Sender')} aria-expanded={props.contact === id} onClick={() => props.onContact(id)}>Info</button></Show>
            </div>;
        }}</For>
    </section>;
}
