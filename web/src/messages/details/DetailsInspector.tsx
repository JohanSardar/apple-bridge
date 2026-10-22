import { Icon } from '../../design/Icon';
import { strings as s } from '../../app/strings';
import { createEffect, createMemo, createSignal, For, onCleanup, Show } from 'solid-js';
import type { LiveClient } from '../../data/live';
import type { ChatSummary } from '../../data/types';
import { createHandlesCache } from '../store/handles';
import { array, fields, text, type Handle } from '../store/types';
import { conversationTitle } from '../format/list';
import { ChatAvatar } from '../list/Avatar';
import { Collections } from './Collections';
import { GroupManagement } from './GroupManagement';
import './details.css';
function links(person:Handle|undefined){
    const address=person?.address?.trim()??'';
    if(/^[^\s@]+@[^\s@]+\.[^\s@]+$/u.test(address))return {mail:'mailto:'+encodeURIComponent(address),face:'facetime:'+encodeURIComponent(address)};
    if(/^\+?[\d ()-]{7,}$/u.test(address))return {call:'tel:'+address.replace(/[^+\d]/gu,''),face:'facetime:'+address.replace(/[^+\d]/gu,'')};
    return {};
}
export default function DetailsInspector(props:{live:LiveClient;chatID:string;onShowInChat?:()=>void}){
    const cache=createHandlesCache(props.live),[contact,setContact]=createSignal<string>();
    const view=props.live.subscribe('record',{collection:'messages.chat',id:props.chatID});
    onCleanup(view.dispose);
    const chat=createMemo<ChatSummary|undefined>(()=>{const record=view.data().records?.[0];if(!record)return;return {chat:record,title:text(record.data.display_name)||cache.read().get(text(array(record.data.participants)[0]))?.display_name||s.conversation,avatar_handles:array(record.data.participants).filter((id):id is string=>typeof id==='string').map(id=>({id,contact_id:null})),last_message:null,last_activity_at:'',unread_count:0,has_unseen_mention:false,muted:null,pinned_index:null};});
    const participants=createMemo(()=>array(chat()?.chat.data.participants).filter((id):id is string=>typeof id==='string'));
    const handles=()=>cache.read(),title=()=>chat()?conversationTitle(chat()!,handles()):s.conversation;
    createEffect(()=>{cache.ensure(participants());});
    const single=()=>participants().length===1?handles().get(participants()[0]):undefined;
    function ContactInfo(info:{person:Handle}){
        const view=props.live.subscribe('record',{collection:'contacts.contact',id:info.person.contact_id});
        onCleanup(view.dispose);
        const record=()=>view.data().records?.[0];
        const entries=()=>[...new Set([info.person.address,...['phones','emails'].flatMap(key=>array(record()?.data[key]).map(value=>typeof value==='string'?value:text(fields(value).value)))].filter((value):value is string=>Boolean(value)))];
        return <section class="details-contact" aria-label={s.contactInformation}><h3>{info.person.display_name}</h3><For each={entries()}>{value=><p>{value}</p>}</For><Show when={text(record()?.data.organization)}><p>{text(record()?.data.organization)}</p></Show></section>;
    }
    return <div class="conversation-details" data-chat-id={props.chatID}><Show when={chat()} fallback={<p role="status">{!view.data().records?s.loadingConversation:s.conversationUnavailable}</p>}>{value=><><header class="details-identity"><ChatAvatar chat={value()} handles={handles()}/><h2>{title()}</h2><p class="secondary">{participants().length>1?s.peopleCount(participants().length):single()?.address}</p><nav class="details-actions" aria-label={s.contactActions}><Show when={links(single()).call}><a href={links(single()).call}><Icon name="phone"/><span>{s.call}</span></a></Show><Show when={links(single()).face}><a href={links(single()).face}><Icon name="video"/><span>{s.faceTime}</span></a></Show><Show when={links(single()).mail}><a href={links(single()).mail}><Icon name="mail"/><span>{s.mail}</span></a></Show><Show when={single()?.contact_id}><button type="button" onClick={()=>setContact(contact()?undefined:single()!.id)} aria-expanded={Boolean(contact())}><Icon name="info"/><span>{s.info}</span></button></Show></nav></header><Show when={participants().length>1}><GroupManagement live={props.live} record={value().chat} handles={handles()} cursor={view.cursor()} contact={contact()} onContact={id=>setContact(contact()===id?undefined:id)}/></Show><Show when={contact()?handles().get(contact()!):undefined} keyed>{person=><ContactInfo person={person}/>}</Show><Collections live={props.live} chatID={props.chatID} onShowInChat={props.onShowInChat}/></>}</Show></div>;
}
