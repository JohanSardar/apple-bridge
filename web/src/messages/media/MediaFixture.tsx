import {For,Show,createSignal,onCleanup} from 'solid-js';
import {auth} from '../../auth/session';
import type {BridgeRecord} from '../../data/types';
import {fields,text} from '../store/types';
import {MessageMedia} from './MessageMedia';
import {ChatMediaViewer} from './ChatMediaViewer';
import {attachmentHasAlpha} from './source';
import {MediaStack} from './MediaStack';
export function MediaFixture(){
 const params=new URLSearchParams(location.search),chatId=params.get('chat')??'';
 const view=auth.live()?.subscribe('thread',{chat_id:chatId,limit:params.has('window')?Math.max(1,Math.min(200,Number(params.get('window'))||1)):200,...(params.get('anchor')?{anchor:params.get('anchor')}:{})});
 const [selected,setSelected]=createSignal<{attachment:BridgeRecord;origin:DOMRectReadOnly}>(),[shown,setShown]=createSignal('');
 onCleanup(()=>view?.dispose());
 const items=()=>(view?.data().items??[]).filter(item=>!item.data.balloon).flatMap(message=>message.attachments.filter(attachment=>text(attachment.data.mime_type).startsWith('image/')&&!attachment.data.hidden&&!attachment.data.genmoji&&!attachment.data.is_sticker).map(attachment=>({message,attachment})));
 const stackIds=params.getAll('stack'),stack=()=>stackIds.flatMap(id=>items().filter(value=>value.message.id===id).map(value=>value.attachment)),phone=matchMedia('(max-width:699px)').matches;
 return <><section class="media-fixture" aria-label="Media bubbles" data-shown-message={shown()}><Show when={stackIds.length>0} fallback={<For each={items()}>{value=>{
  const dimensions=fields(value.attachment.data.media),width=Number(dimensions.width)||260,height=Number(dimensions.height)||120,scale=Math.min(1,(matchMedia('(max-width:699px)').matches?240:320)/width,360/height);
  return <div class="media-fixture-slot" data-media-origin={value.attachment.id} data-media-alpha={attachmentHasAlpha(value.attachment)} data-message={value.message.id} style={{width:`${width*scale}px`,height:`${height*scale}px`}}><MessageMedia message={value.message} part={{index:0,kind:'attachment',attachment_id:value.attachment.id}} width={width*scale} height={height*scale} onOpen={(attachment,origin)=>setSelected({attachment,origin})}/></div>;
 }}</For>}><div class="media-fixture-slot" style={{width:phone?'260px':'360px',height:phone?'320px':'280px'}}><MediaStack attachments={stack()} width={phone?260:360} height={phone?320:280} onOpen={(attachment,origin)=>setSelected({attachment,origin})}/></div></Show></section><Show keyed when={selected()}>{value=><ChatMediaViewer chatId={chatId} attachment={value.attachment} origin={value.origin} pageSize={Number(params.get('pageSize'))||200} onClose={()=>setSelected(undefined)} onShowInChat={attachment=>setShown(text(attachment.data.message_id))}/>}</Show></>;
}
