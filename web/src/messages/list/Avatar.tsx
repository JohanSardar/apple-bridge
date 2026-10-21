import { createEffect, createMemo, createSignal, For, onCleanup, Show } from 'solid-js';
import { auth } from '../../auth/session';
import type { BridgeRecord, ChatSummary } from '../../data/types';
import type { Handles } from '../store/types';
import { fields, text } from '../store/types';
import { initials } from '../format';
export function Avatar(props:{title:string;handle?:string;handles:Handles;photo?:{digest:string;record:string};class?:string}) {
    const [failed,setFailed]=createSignal('');
    const person=()=>props.handle?props.handles.get(props.handle):undefined;
    const photo=()=>props.photo??(person()?.avatar?{digest:person()!.avatar!.digest,record:person()!.contact_id??props.handle!}:undefined);
    const url=createMemo(()=>{const value=photo();return value?`/web/v1/blob/${encodeURIComponent(value.digest)}?record=${encodeURIComponent(value.record)}&variant=thumb`:'';});
    return <span class={'avatar '+(props.class??'')} aria-hidden="true"><Show when={url()&&failed()!==url()} fallback={<span>{person()?.initials||initials(props.title)||'?'}</span>}><img src={url()} alt="" decoding="async" loading="lazy" onError={()=>setFailed(url())}/></Show></span>;
}
export function ChatAvatar(props:{chat:ChatSummary;handles:Handles}) {
    const [groupPhoto,setGroupPhoto]=createSignal<BridgeRecord>();
    const groupPhotoID=createMemo(()=>text(props.chat.chat.data.group_photo_attachment_id));
    createEffect(()=>{const id=groupPhotoID(),live=auth.live();setGroupPhoto(undefined);if(!id||!live)return;const subscription=live.subscribe('record',{collection:'messages.attachment',id});createEffect(()=>setGroupPhoto(subscription.data().records?.[0]));onCleanup(subscription.dispose);});
    const members=()=>props.chat.avatar_handles??[];
    const photo=()=>{const record=groupPhoto();const value=fields(record?.data.blob??props.chat.chat.data.group_photo);return text(value.digest)?{digest:text(value.digest),record:record?.id??props.chat.chat.id}:undefined;};
    return <Show when={members().length>1&&!photo()} fallback={<Avatar title={props.chat.title} handle={members()[0]?.id} handles={props.handles} photo={photo()}/>}><span class="avatar avatar-group" data-count={Math.min(members().length,3)} aria-hidden="true"><For each={members().slice(0,3)}>{(member,index)=><Avatar class={'avatar-member member-'+index()} title={props.handles.get(member.id)?.display_name??props.chat.title} handle={member.id} handles={props.handles}/>}</For></span></Show>;
}
