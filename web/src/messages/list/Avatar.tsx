import { createEffect, createMemo, createSignal, For, onCleanup, Show } from 'solid-js';
import { auth } from '../../auth/session';
import type { BridgeRecord, ChatSummary } from '../../data/types';
import type { Handles } from '../store/types';
import { fields, text } from '../store/types';
import { avatarInitials, conversationTitle } from '../format/list';
export function Avatar(props:{title:string;handle?:string;handles:Handles;photo?:{digest:string;record:string};variant?:'thumb'|'display';class?:string;nameReady?:boolean}) {
    const [failed,setFailed]=createSignal(''),[loaded,setLoaded]=createSignal('');
    const person=()=>props.handle?props.handles.get(props.handle):undefined;
    const photo=()=>props.photo??(person()?.avatar?{digest:person()!.avatar!.digest,record:person()!.contact_id??props.handle!}:undefined);
    const monogram=createMemo(()=>props.nameReady===false||props.handle&&person()?.name_resolved!==true?'':avatarInitials(props.title,person()?.given_name,person()?.family_name));
    const url=createMemo(()=>{const value=photo();return value?`/web/v1/blob/${encodeURIComponent(value.digest)}?record=${encodeURIComponent(value.record)}&variant=${props.variant??'thumb'}`:'';});
    return <span class={'avatar '+(props.class??'')} aria-hidden="true" classList={{'avatar-has-monogram':Boolean(monogram())}}><svg class="avatar-person" viewBox="0 0 40 40"><circle cx="20" cy="14" r="7" fill="currentColor"/><path d="M6 39v-3c0-8 6-13 14-13s14 5 14 13v3Z" fill="currentColor"/></svg><span class="avatar-monogram">{monogram()}</span><Show when={url()&&failed()!==url()}><img src={url()} alt="" decoding="async" loading="lazy" classList={{'avatar-image-ready':loaded()===url()}} onLoad={event=>{if(event.currentTarget.naturalWidth<=1||event.currentTarget.naturalHeight<=1)setFailed(url());else setLoaded(url());}} onError={()=>setFailed(url())}/></Show></span>;
}
export function ChatAvatar(props:{chat:ChatSummary;handles:Handles}) {
    const [groupPhoto,setGroupPhoto]=createSignal<BridgeRecord>();
    const groupPhotoID=createMemo(()=>text(props.chat.chat.data.group_photo_attachment_id));
    createEffect(()=>{const id=groupPhotoID(),live=auth.live();setGroupPhoto(undefined);if(!id||!live)return;const subscription=live.subscribe('record',{collection:'messages.attachment',id});createEffect(()=>setGroupPhoto(subscription.data().records?.[0]));onCleanup(subscription.dispose);});
    const members=()=>props.chat.avatar_handles??[];
    const nameReady=()=>Boolean(text(props.chat.chat.data.display_name))||(Array.isArray(props.chat.chat.data.participants)?props.chat.chat.data.participants:[]).every(id=>typeof id==='string'&&props.handles.get(id)?.name_resolved===true);
    const photo=()=>{const record=groupPhoto();const value=fields(record?.data.blob??props.chat.chat.data.group_photo);return text(value.digest)?{digest:text(value.digest),record:record?.id??props.chat.chat.id}:undefined;};
    return <Show when={members().length>1&&!photo()} fallback={<Avatar title={conversationTitle(props.chat,props.handles)} handle={photo()?undefined:members()[0]?.id} handles={props.handles} photo={photo()} nameReady={nameReady()}/>}><span class="avatar avatar-group" data-count={Math.min(members().length,3)} aria-hidden="true"><For each={members().slice(0,3)}>{(member,index)=><Avatar class={'avatar-member member-'+index()} title={props.handles.get(member.id)?.display_name??props.chat.title} handle={member.id} handles={props.handles}/>}</For></span></Show>;
}
