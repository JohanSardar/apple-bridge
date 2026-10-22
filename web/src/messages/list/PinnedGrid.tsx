import { strings as s } from '../../app/strings';
import { For, Show } from 'solid-js';
import type { ChatSummary } from '../../data/types';
import type { Handles } from '../store/types';
import { ChatAvatar } from './Avatar';
import { conversationTitle } from '../format/list';
export function PinnedGrid(props:{chats:readonly ChatSummary[];handles:Handles;selected?:string;onOpen:(chat:ChatSummary)=>void}) {
    return <div class="pinned-grid" role="group" aria-label={s.pinnedConversations}><For each={props.chats}>{chat=><button class="pinned-conversation" aria-label={conversationTitle(chat,props.handles)} aria-pressed={props.selected===chat.chat.id} onClick={()=>props.onOpen(chat)}><Show when={chat.unread_count>0?chat.pin_preview:undefined}>{value=><span class="pin-preview"><span>{value().text_preview}</span></span>}</Show><ChatAvatar chat={chat} handles={props.handles}/><span class="pin-name">{conversationTitle(chat,props.handles)}</span></button>}</For></div>;
}
