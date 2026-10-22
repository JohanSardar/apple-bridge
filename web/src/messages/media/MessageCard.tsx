import {createMemo,createSignal,ErrorBoundary,lazy,onCleanup,Show,Suspense} from 'solid-js';
import type {ThreadItem} from '../../data/types';
import {reducedMotion,springAnimation} from '../../motion/animate';
import {text} from '../store/types';
import {MediaImage} from './MediaImage';
import {messageCardData} from './cards';
import {mediaSource} from './source';
import './cards.css';
const CardVideo=lazy(()=>import('./CardVideo').then(module=>({default:module.CardVideo})));
export function MessageCard(props:{message:ThreadItem}) {
    const data=createMemo(()=>messageCardData(props.message)),[playing,setPlaying]=createSignal(false);
    let control:HTMLButtonElement|undefined,motion:ReturnType<typeof springAnimation>|undefined;
    const press=(target:number)=>{if(!control)return;if(reducedMotion()){motion?.cancel();motion=undefined;control.style.transform='';return;}if(motion)motion.retarget(target);else motion=springAnimation(value=>{control!.style.transform=`scale(${value})`;},1,target,{response:.26,damping:.8});};
    onCleanup(()=>motion?.cancel());
    const asset=createMemo(()=>props.message.attachments.find(value=>value.id===data()?.imageAttachmentId));
    const icon=createMemo(()=>props.message.attachments.find(value=>value.id===data()?.iconAttachmentId&&text(value.data.mime_type).startsWith('image/')));
    const video=createMemo(()=>{const value=asset();return value&&text(value.data.mime_type).startsWith('video/')&&mediaSource(value,'video')?value:undefined;});
    return <Show when={data()}>{card=><div class="message-card" data-card-kind={card().kind} data-card-image={card().hasImage}>
        <Show when={card().hasImage}><div class="message-card-art"><Show when={asset()} fallback={<span class="message-card-missing" role="status">Preview unavailable</span>}>{attachment=><MediaImage attachment={attachment()} variant={video()?'poster':'thumb'} alt=""/>}</Show>
            <Show when={playing()&&video()}>{attachment=><ErrorBoundary fallback={<span class="message-card-missing" role="status">Video unavailable</span>}><Suspense fallback={<span class="message-card-missing" role="status">Opening video…</span>}><CardVideo attachment={attachment()}/></Suspense></ErrorBoundary>}</Show>
        </div></Show>
        <div class="message-card-copy"><Show when={card().kind==='app'&&card().appName.trim().toLowerCase()!==card().title.trim().toLowerCase()}><span class="message-card-name">{card().appName}</span></Show><strong class="message-card-title">{card().title}</strong><Show when={card().kind==='link'} fallback={<span class="message-card-device">Open on an Apple device</span>}><span class="message-card-domain">{card().domain||'Link'}</span></Show></div>
        <Show when={card().url}><a class="message-card-target" href={card().url} target="_blank" rel="noopener noreferrer" aria-label={card().title}/></Show>
        <Show when={!card().hasImage&&(card().iconAttachmentId||card().video)}><span class="message-card-trailing" data-card-icon={Boolean(icon())}><Show when={icon()}>{attachment=><MediaImage attachment={attachment()} alt=""/>}</Show><Show when={card().video}><span class="message-card-video-badge" role="img" aria-label="Video link"><PlayGlyph/></span></Show></span></Show>
        <Show when={card().hasImage&&card().video&&!playing()}><Show when={video()} fallback={<span class="message-card-video-badge" role="img" aria-label="Video link"><PlayGlyph/></span>}><button type="button" class="message-card-play" ref={control} aria-label="Play video preview" onPointerDown={event=>{if(event.button===0)press(.92);}} onPointerUp={()=>press(1)} onPointerLeave={()=>press(1)} onPointerCancel={()=>press(1)} onKeyDown={event=>{if(event.key===' ')press(.92);}} onKeyUp={()=>press(1)} onBlur={()=>press(1)} onClick={()=>{motion?.cancel();motion=undefined;control=undefined;setPlaying(true);}}><PlayGlyph/></button></Show></Show>
    </div>}</Show>;
}
function PlayGlyph(){return <svg viewBox="0 0 24 24" aria-hidden="true"><use width="24" height="24" href="/icons.svg#message-card-play"/></svg>;}
