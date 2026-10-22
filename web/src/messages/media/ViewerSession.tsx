import {Show,onMount,onCleanup,createSignal,createEffect,createMemo} from 'solid-js';
import {auth} from '../../auth/session';
import type {BridgeRecord} from '../../data/types';
import {createMediaCollection} from './collection';
import {MediaViewer} from './MediaViewer';
import type {ChatMediaViewerProps} from './ChatMediaViewer';
import {createHandlesCache} from '../store/handles';
import {text} from '../store/types';
import {header,name,wording} from '../format';
export function ViewerSession(props:ChatMediaViewerProps){
    const sources=new Map<HTMLElement,string>();
    const [selected,setSelected]=createSignal(props.attachment);
    const restore=()=>{for(const [element,visibility] of sources)element.style.visibility=visibility;sources.clear();};
    const source=(attachment:BridgeRecord)=>[...document.querySelectorAll<HTMLElement>(`[data-media-origin="${CSS.escape(attachment.id)}"],[data-media-id="${CSS.escape(attachment.id)}"]`)].find(element=>!element.closest('.media-viewer')&&!element.closest('[inert]'));
    const hide=(element?:HTMLElement)=>{if(element){sources.set(element,element.style.visibility);element.style.visibility='hidden';}};
    const originFor=(attachment:BridgeRecord)=>{
        restore();const image=source(attachment),rect=props.originFor?props.originFor(attachment):image?.getBoundingClientRect();
        if(rect&&rect.width>0&&rect.height>0&&rect.bottom>0&&rect.top<innerHeight&&rect.right>0&&rect.left<innerWidth){hide(image);return rect;}
    };
    onMount(()=>hide(source(props.attachment)));onCleanup(restore);
    const close=()=>{restore();const element=source(props.attachment),trigger=element?.querySelector<HTMLElement>('.media-open')??element?.closest<HTMLElement>('button,a')??element?.closest('.media-bubble')?.querySelector<HTMLElement>('.media-open');trigger?.focus({preventScroll:true});props.onClose();};
    const shareUrl=(attachment:BridgeRecord)=>new URL(`/messages/${encodeURIComponent(props.chatId)}/${encodeURIComponent(String(attachment.data.message_id??''))}`,location.origin).href;
    return <Show keyed when={auth.live()}>{live=>{
        const collection=createMediaCollection(live,props.chatId,Math.max(1,Math.min(200,props.pageSize??200))),handles=createHandlesCache(live),cache=new Map<string,BridgeRecord>();
        const [owner,setOwner]=createSignal<BridgeRecord>(),[record,setRecord]=createSignal<ReturnType<typeof live.subscribe>>();
        const ownerId=createMemo(()=>text(selected().data.message_id));
        createEffect(()=>{const id=ownerId();setOwner(cache.get(id));setRecord(undefined);if(!id)return;let subscription:ReturnType<typeof live.subscribe>|undefined;const timer=setTimeout(()=>{subscription=live.subscribe('record',{collection:'messages.message',id});setRecord(subscription);},80);onCleanup(()=>{clearTimeout(timer);subscription?.dispose();});});
        createEffect(()=>{const message=record()?.data().records?.find(value=>value.id===ownerId());if(message){cache.delete(message.id);cache.set(message.id,message);if(cache.size>64)cache.delete(cache.keys().next().value!);setOwner(message);}});
        createEffect(()=>{const sender=text(owner()?.data.sender_handle_id);if(sender)handles.ensure([sender]);});
        const timestamp=()=>text(owner()?.data.sent_at)||text(selected().data.sent_at);
        const subtitle=()=>Number.isFinite(Date.parse(timestamp()))?header(timestamp(),props.now,props.locale):'';
        const senderName=()=>owner()?.data.is_from_me?wording.you:name(text(owner()?.data.sender_handle_id),handles.read());
        return <MediaViewer {...props} onClose={close} onSelection={value=>setSelected(value)} senderName={senderName()} timestamp={timestamp()} subtitle={subtitle()} originFor={originFor} shareUrl={shareUrl} attachments={collection.attachments()} loading={collection.loading()} error={collection.error()} onRetry={collection.retry}/>;
    }}</Show>;
}
