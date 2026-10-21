import {Show,onMount,onCleanup} from 'solid-js';
import {auth} from '../../auth/session';
import type {BridgeRecord} from '../../data/types';
import {createMediaCollection} from './collection';
import {MediaViewer,type MediaViewerProps} from './MediaViewer';
export type ChatMediaViewerProps=Omit<MediaViewerProps,'attachments'|'loading'|'error'|'onRetry'|'shareUrl'> & {chatId:string;pageSize?:number};
export function ChatMediaViewer(props:ChatMediaViewerProps){
    const sources=new Map<HTMLElement,string>();
    const restore=()=>{for(const [element,visibility] of sources)element.style.visibility=visibility;sources.clear();};
    const source=(attachment:BridgeRecord)=>[...document.querySelectorAll<HTMLElement>(`[data-media-id="${CSS.escape(attachment.id)}"]`)].find(element=>!element.closest('.media-viewer'));
    const hide=(element?:HTMLElement)=>{if(element){sources.set(element,element.style.visibility);element.style.visibility='hidden';}};
    const originFor=(attachment:BridgeRecord)=>{
        restore();const image=source(attachment),rect=props.originFor?props.originFor(attachment):image?.getBoundingClientRect();
        if(rect&&rect.width>0&&rect.height>0&&rect.bottom>0&&rect.top<innerHeight&&rect.right>0&&rect.left<innerWidth){hide(image);return rect;}
    };
    onMount(()=>hide(source(props.attachment)));onCleanup(restore);
    const shareUrl=(attachment:BridgeRecord)=>new URL(`/messages/${encodeURIComponent(props.chatId)}/${encodeURIComponent(String(attachment.data.message_id??''))}`,location.origin).href;
    return <Show keyed when={auth.live()}>{live=>{const collection=createMediaCollection(live,props.chatId,Math.max(1,Math.min(200,props.pageSize??200)));return <MediaViewer {...props} originFor={originFor} shareUrl={shareUrl} attachments={collection.attachments()} loading={collection.loading()} error={collection.error()} onRetry={collection.retry}/>;}}</Show>;
}
