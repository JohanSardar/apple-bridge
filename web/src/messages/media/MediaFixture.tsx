import {FilesFixture} from './FilesFixture';
import {AudioFixture} from './AudioFixture';
import {For,Show,createSignal,onCleanup} from 'solid-js';
import {PendingFixture} from './PendingFixture';
import {EffectsFixture} from '../effects/EffectsFixture';
import {auth} from '../../auth/session';
import type {BridgeRecord,ThreadItem} from '../../data/types';
import {fields,text} from '../store/types';
import {MessageMedia} from './MessageMedia';
import {ChatMediaViewer} from './ChatMediaViewer';
import {attachmentHasAlpha} from './source';
import {MediaStack} from './MediaStack';
import {StickerFixture} from './StickerFixture';
import {PlaceholderFixture} from './PlaceholderFixture';
export function MediaFixture(){
 const params=new URLSearchParams(location.search),chatId=params.get('chat')??'';
 const view=auth.live()?.subscribe('thread',{chat_id:chatId,limit:params.has('window')?Math.max(1,Math.min(200,Number(params.get('window'))||1)):200,...(params.get('anchor')?{anchor:params.get('anchor')}:{})});
 const [selected,setSelected]=createSignal<{attachment:BridgeRecord;origin:DOMRectReadOnly}>(),[shown,setShown]=createSignal('');
 onCleanup(()=>view?.dispose());
 const items=()=>(view?.data().items??[]).filter(item=>!item.data.balloon).flatMap(message=>message.attachments.filter(attachment=>(params.has('motion')?(text(attachment.data.mime_type).startsWith('video/')||fields(attachment.data.media).live===true):text(attachment.data.mime_type).startsWith('image/'))&&!attachment.data.hidden&&!attachment.data.genmoji&&!attachment.data.is_sticker).map(attachment=>({message,attachment})));
 const stackIds=params.getAll('stack'),stack=()=>stackIds.flatMap(id=>items().filter(value=>value.message.id===id).map(value=>value.attachment)),phone=matchMedia('(max-width:699px)').matches;
 if(params.has('stickers'))return <StickerFixture items={view?.data().items??[]} standalone={params.get('stickers')==='standalone'}/>;
 if(params.has('placeholders'))return <PlaceholderFixture items={view?.data().items??[]} mode={params.get('placeholders')??'1'}/>;
 if(params.has('audio'))return <AudioFixture items={view?.data().items??[]} plain={params.get('audio')==='plain'}/>;
 if(params.has('files'))return <FilesFixture items={view?.data().items??[]} pending={params.get('files')==='pending'}/>;
 if(params.has('effects'))return <EffectsFixture items={view?.data().items??[]}/>;
 if(params.has('pending'))return <PendingFixture items={view?.data().items??[]} mode={params.get('pending')??'image'}/>;
 if(params.has('cards')){
  const messages=():ThreadItem[]=>{const items=view?.data().items??[];if(params.get('cards')==='video'){const message=items.find(value=>value.attachments.some(attachment=>text(attachment.data.mime_type).startsWith('video/')));return message?[{...message,data:{...message.data,balloon:{kind:'link',link:{url:'https://example.com/local-video',title:'Local video preview',image_attachment_id:message.attachments.find(value=>text(value.data.mime_type).startsWith('video/'))?.id,video_url:'https://example.com/remote-video.mp4'}}}}]:[];}return items.filter(message=>['link','app'].includes(text(fields(message.data.balloon).kind))).map(message=>{if(params.get('cards')==='case'&&fields(message.data.balloon).kind==='app')return {...message,data:{...message.data,balloon:{...fields(message.data.balloon),app_name:' DEMO APP '}}};if(!['plain','invalid','icon'].includes(params.get('cards')??''))return message;const balloon=fields(message.data.balloon),kind=text(balloon.kind),payload=fields(balloon[kind]);return {...message,data:{...message.data,balloon:{...balloon,[kind]:{...payload,image_attachment_id:undefined,icon_attachment_id:params.get('cards')==='icon'?payload.icon_attachment_id:undefined,...(params.get('cards')==='invalid'?{url:'javascript:alert(1)'}:{})}}}};});};
  return <section class="media-fixture" aria-label="Message cards"><For each={messages()}>{message=>{const balloon=fields(message.data.balloon),payload=fields(balloon[text(balloon.kind)]),height=text(payload.image_attachment_id)?180:88;return <div class="media-fixture-slot" data-message={message.id} style={{width:'280px',height:height+'px'}}><MessageMedia message={message} part={{index:0,kind:'balloon'}} width={280} height={height}/></div>;}}</For></section>;
 }
 return <><section class="media-fixture" aria-label="Media bubbles" data-shown-message={shown()}><Show when={stackIds.length>0} fallback={<For each={items()}>{value=>{
  const dimensions=fields(value.attachment.data.media),width=Number(dimensions.width)||260,height=Number(dimensions.height)||120,maximum=phone?240:320,scale=Math.min(1,maximum/width,360/height),minimum=params.has('motion')?120:0,boxWidth=Math.min(maximum,Math.max(minimum,width*scale)),boxHeight=Math.min(360,Math.max(minimum,height*scale));
  return <div class="media-fixture-slot" data-media-origin={value.attachment.id} data-media-alpha={attachmentHasAlpha(value.attachment)} data-message={value.message.id} style={{width:`${boxWidth}px`,height:`${boxHeight}px`}}><MessageMedia message={value.message} part={{index:0,kind:'attachment',attachment_id:value.attachment.id}} width={boxWidth} height={boxHeight} onOpen={(attachment,origin)=>setSelected({attachment,origin})}/></div>;
 }}</For>}><div class="media-fixture-slot" style={{width:phone?'260px':'360px',height:phone?'320px':'280px'}}><MediaStack attachments={stack()} width={phone?260:360} height={phone?320:280} onOpen={(attachment,origin)=>setSelected({attachment,origin})}/></div></Show></section><Show keyed when={selected()}>{value=><ChatMediaViewer chatId={chatId} attachment={value.attachment} origin={value.origin} pageSize={Number(params.get('pageSize'))||200} onClose={()=>setSelected(undefined)} onShowInChat={attachment=>setShown(text(attachment.data.message_id))}/>}</Show></>;
}
