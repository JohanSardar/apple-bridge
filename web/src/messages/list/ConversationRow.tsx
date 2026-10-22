import { strings as s } from '../../app/strings';
import { createEffect, onCleanup, Show } from 'solid-js';
import type { ChatSummary } from '../../data/types';
import type { Handles } from '../store/types';
import { conversationDate, preview, explicitlyMuted, conversationTitle } from '../format/list';
import { ChatAvatar } from './Avatar';
import { Icon } from '../../design/Icon';
import { crossfade, reducedMotion, springAnimation } from '../../motion/animate';
export function ConversationRow(props:{chat:ChatSummary;handles:Handles;selected:boolean;unreadPulse?:number;onOpen:(chat:ChatSummary,event:MouseEvent)=>void}) {
    let dot:HTMLSpanElement|undefined,played=0;
    let animation:ReturnType<typeof springAnimation>|undefined,fade:Animation|undefined;
    createEffect(()=>{const pulse=props.unreadPulse??0;if(pulse>played&&performance.now()-pulse<600&&props.chat.unread_count>0&&dot){played=pulse;animation?.cancel();fade?.cancel();if(reducedMotion())fade=crossfade(dot);else animation=springAnimation(value=>{if(dot)dot.style.transform=`scale(${value})`;},.15,1,{response:.32,damping:.7});}});
    onCleanup(()=>{animation?.cancel();fade?.cancel();});
    const muted=()=>explicitlyMuted(props.chat.muted);
    return <a draggable={false} class="conversation-row" data-chat={props.chat.chat.id} href={'/messages/'+encodeURIComponent(props.chat.chat.id)} data-selected={props.selected} tabindex="-1" onClick={event=>{if(event.metaKey||event.ctrlKey||event.shiftKey||event.altKey)return;event.preventDefault();props.onOpen(props.chat,event);}}><span class="conversation-unread"><Show when={props.chat.unread_count>0}><span ref={dot} class="unread-dot" aria-label="Unread"/></Show></span><ChatAvatar chat={props.chat} handles={props.handles}/><span class="conversation-content"><span class="conversation-line"><span class="conversation-name">{conversationTitle(props.chat,props.handles)||s.unknownSender}</span><Show when={muted()}><svg class="muted-icon" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.4" aria-label={s.notificationsSilenced}><title>{s.notificationsSilenced}</title><path d="M5 5.5C5 3 7 2 10 2s5 2 5 5v4l2 3H9M6 14H3l2-3V9m3 7c1 2 3 2 4 0M2 2l16 16"/></svg></Show><time datetime={props.chat.last_activity_at} title={new Date(props.chat.last_activity_at).toLocaleString()}>{conversationDate(props.chat.last_activity_at)}</time><Icon name="chevron" class="conversation-chevron"/></span><span class="conversation-preview">{preview(props.chat.last_message,props.handles)}</span><Show when={props.chat.has_unseen_mention}><span class="mention-badge" aria-label={s.unseenMention}>@</span></Show></span></a>;
}
