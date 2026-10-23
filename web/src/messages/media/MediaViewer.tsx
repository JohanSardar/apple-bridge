import { createEffect, createMemo, createSignal, For, onCleanup, untrack, Show } from 'solid-js';
import type { BridgeRecord } from '../../data/types';
import { fields, text } from '../store/types';
import { springAnimation, reducedMotion } from '../../motion/animate';
import { MediaImage } from './MediaImage';
import { VideoPlayer } from './PlayerControls';
import { attachmentHasAlpha, mediaSource } from './source';
import './viewer.css';
import {attachViewerChrome} from './chrome';
export type MediaViewerProps={open?:boolean;senderName?:string;timestamp?:string;subtitle?:string;onSelection?:(attachment:BridgeRecord)=>void;loading?:boolean;error?:boolean;onRetry?:()=>void;shareUrl?:(attachment:BridgeRecord)=>string;attachments:readonly BridgeRecord[];attachment:BridgeRecord;origin:DOMRectReadOnly;onClose:()=>void;onShowInChat?:(attachment:BridgeRecord)=>void;originFor?:(attachment:BridgeRecord)=>DOMRectReadOnly|undefined};
export function MediaViewer(props:MediaViewerProps) {
    const items=()=>props.attachments;
    const clamp=(value:number,min=0,max=1)=>Math.max(min,Math.min(max,value));
    const control=(event:Event,selector='button,a,input')=>(event.target as HTMLElement).closest(selector);
    const [selectedId,setSelectedId]=createSignal(props.attachment.id),[zoom,setZoom]=createSignal(1),[status,setStatus]=createSignal(''),[livePlaying,setLivePlaying]=createSignal(false),[previewsReady,setPreviewsReady]=createSignal(false),[viewport,setViewport]=createSignal({width:innerWidth,height:innerHeight});
    const index=createMemo(()=>items().findIndex(value=>value.id===selectedId()));
    const current=createMemo(()=>items()[index()]??props.attachment);
    let dialog!:HTMLDialogElement,stage!:HTMLElement,backdrop!:HTMLDivElement,video:HTMLVideoElement|undefined;
    let animation:ReturnType<typeof springAnimation>|undefined,progress=0,closing=false,panX=0,panY=0,dragX=0,dragY=0,axis='',distance=0,pinchZoom=1;
    const previews:HTMLDivElement[]=[];
    const pointers=new Map<number,{x:number;y:number}>();
    let start={x:0,y:0,panX:0,panY:0},last={x:0,y:0,at:0},velocity={x:0,y:0};
    const mediaBox=(attachment:BridgeRecord)=>{const dimensions=fields(attachment.data.media),w=Number(dimensions.width)||1,h=Number(dimensions.height)||1,{width,height}=viewport(),scale=Math.min(width/w,Math.max(1,height-120)/h);return {x:(width-w*scale)/2,y:(height-h*scale)/2,width:w*scale,height:h*scale};};
    const box=createMemo(()=>mediaBox(current()));
    const boxStyle=({x,y,width,height}:ReturnType<typeof mediaBox>)=>({left:x+'px',top:y+'px',width:width+'px',height:height+'px'});
    let targetOrigin:DOMRectReadOnly|undefined;
    const origin=()=>closing?targetOrigin:current().id===props.attachment.id?props.origin:undefined;
    const paint=()=>{
        for(let i=0;i<previews.length;i++){const preview=previews[i];preview.style.transform=`translate3d(${(i===0?-1:1)*viewport().width+dragX}px,0,0)`;preview.style.opacity=zoom()===1&&axis==='x'?'1':'0';}const bounds=box(),source=origin(),alpha=attachmentHasAlpha(current()),p=clamp(progress),moving=!reducedMotion()&&source;
        const x=moving?(source.x+source.width/2-bounds.x-bounds.width/2)*(1-p):0,y=moving?(source.y+source.height/2-bounds.y-bounds.height/2)*(1-p):0;
        const initialScale=moving?(alpha?Math.min:Math.max)(source.width/bounds.width,source.height/bounds.height):1,scale=initialScale*(1-p)+p;
        const cropX=moving?Math.max(0,(bounds.width-source.width/initialScale)/2)*(1-p):0,cropY=moving?Math.max(0,(bounds.height-source.height/initialScale)/2)*(1-p):0;
        const dismiss=clamp(dragY/(viewport().height*.3)),dismissScale=reducedMotion()?1:1-.15*dismiss*p;
        stage.style.transform=`translate3d(${x+dragX*p+panX*p}px,${y+dragY*p+panY*p}px,0) scale(${scale*(1+(zoom()-1)*p)*dismissScale})`;
        stage.style.clipPath=`inset(${cropY}px ${cropX}px round ${alpha?0:18*((1-p)+dismiss*p)/Math.max(.01,scale*dismissScale)}px)`;backdrop.style.opacity=String(p*(1-.7*dismiss));dialog.style.setProperty('--media-motion-opacity',String(p*Math.max(0,1-dismiss/.15)));
    };
    const settle=(done?:()=>void)=>{animation?.cancel();animation=springAnimation(value=>{progress=value;paint();},progress,closing?0:1,{response:.42,damping:.82},0,done);};
    const close=(after?:()=>void)=>{
        if(closing)return;closing=true;axis='';targetOrigin=props.originFor?props.originFor(current()):current().id===props.attachment.id?props.origin:undefined;video?.pause();animation?.cancel();
        const done=()=>{dialog.close();props.onClose();after?.();};
        if(reducedMotion()||!targetOrigin){const fade=dialog.animate([{opacity:1},{opacity:0}],{duration:150,fill:'forwards'});void fade.finished.then(done).catch(()=>{});}else settle(done);
    };
    const change=(delta:number)=>{if(index()<0){dragX=0;dragY=0;paint();return;}const next=clamp(index()+delta,0,items().length-1);if(next===index()){dragX=0;dragY=0;paint();return;}video?.pause();video=undefined;axis='x';progress=1;setSelectedId(items()[next].id);setLivePlaying(false);setZoom(1);panX=0;panY=0;dragY=0;const from=delta*viewport().width+dragX;animation?.cancel();animation=springAnimation(value=>{dragX=value;paint();},reducedMotion()?0:from,0,{response:.35,damping:1});};
    const zoomAt=(value:number,x:number,y:number)=>{const next=clamp(value,1,4),previous=zoom(),bounds=box(),anchorX=x-bounds.x-bounds.width/2,anchorY=y-bounds.y-bounds.height/2;panX=anchorX+(panX-anchorX)*next/previous;panY=anchorY+(panY-anchorY)*next/previous;setZoom(next);if(next===1){panX=0;panY=0;}paint();};
    const zoomTo=(value:number,x:number,y:number)=>{animation?.cancel();if(reducedMotion()){zoomAt(value,x,y);return;}animation=springAnimation(next=>zoomAt(next,x,y),zoom(),value,{response:.32,damping:1});};
    const down=(event:PointerEvent)=>{
        if(closing||control(event))return;
        setPreviewsReady(true);
        animation?.cancel();progress=1;try{dialog.setPointerCapture(event.pointerId);}catch{}pointers.set(event.pointerId,{x:event.clientX,y:event.clientY});
        if(pointers.size===1){start={x:event.clientX,y:event.clientY,panX,panY};last={x:event.clientX,y:event.clientY,at:event.timeStamp};axis='';velocity={x:0,y:0};}
        if(pointers.size===2){const [a,b]=[...pointers.values()];distance=Math.hypot(a.x-b.x,a.y-b.y);pinchZoom=zoom();dragX=0;dragY=0;}
    };
    const move=(event:PointerEvent)=>{
        if(!pointers.has(event.pointerId)||closing)return;
        pointers.set(event.pointerId,{x:event.clientX,y:event.clientY});
        if(pointers.size===2){const[a,b]=[...pointers.values()];if(distance>0)zoomAt(pinchZoom*Math.hypot(a.x-b.x,a.y-b.y)/distance,(a.x+b.x)/2,(a.y+b.y)/2);return;}
        const dx=event.clientX-start.x,dy=event.clientY-start.y,dt=Math.max(1,event.timeStamp-last.at);velocity={x:(event.clientX-last.x)/dt*1000,y:(event.clientY-last.y)/dt*1000};last={x:event.clientX,y:event.clientY,at:event.timeStamp};
        if(zoom()>1){panX=start.panX+dx;panY=start.panY+dy;}else{if(!axis&&Math.hypot(dx,dy)>8)axis=Math.abs(dx)>Math.abs(dy)?'x':'y';if(axis==='x')dragX=dx;else if(axis==='y')dragY=dy;}paint();
    };
    const up=(event:PointerEvent)=>{
        if(!pointers.delete(event.pointerId))return;if(dialog.hasPointerCapture(event.pointerId))dialog.releasePointerCapture(event.pointerId);
        if(pointers.size){const point=[...pointers.values()][0];start={...point,panX,panY};distance=0;return;}
        if(event.type!=='pointercancel'&&zoom()===1&&(dragY>100||dragY>20&&velocity.y>600)){close();return;}
        if(event.type!=='pointercancel'&&zoom()===1&&axis==='x'&&(Math.abs(dragX)>viewport().width*.18||Math.abs(dragX)>20&&Math.abs(velocity.x)>600)){change(dragX<0?1:-1);return;}
        const bounds=box(),maxX=Math.max(0,(bounds.width*zoom()-viewport().width)/2),maxY=Math.max(0,(bounds.height*zoom()-(viewport().height-120))/2);const initialPanX=panX,initialPanY=panY,targetPanX=clamp(panX,-maxX,maxX),targetPanY=clamp(panY,-maxY,maxY);const fromX=dragX,fromY=dragY;animation?.cancel();if(reducedMotion()){panX=targetPanX;panY=targetPanY;dragX=0;dragY=0;paint();return;}animation=springAnimation(value=>{panX=initialPanX+(targetPanX-initialPanX)*value;panY=initialPanY+(targetPanY-initialPanY)*value;dragX=fromX*(1-value);dragY=fromY*(1-value);paint();},0,1,{response:.35,damping:1});
    };
    const wheel=(event:WheelEvent)=>{if(control(event,'input'))return;event.preventDefault();zoomAt(zoom()*Math.exp(-event.deltaY*.002),event.clientX,event.clientY);};
    const resize=()=>{setViewport({width:innerWidth,height:innerHeight});paint();};
    const share=async()=>{try{await navigator.share({title:text(current().data.filename)||'Photo',url:props.shareUrl?.(current())??location.href});}catch(error){if(!(error instanceof DOMException&&error.name==='AbortError'))setStatus('Sharing unavailable');}};
    const key=(event:KeyboardEvent)=>{if(event.key==='Escape'){event.preventDefault();close();return;}if(control(event,'input'))return;if(event.key==='ArrowRight'){event.preventDefault();change(1);}if(event.key==='ArrowLeft'){event.preventDefault();change(-1);}if(event.key===' '&&!control(event,'button,a')){const player=stage.querySelector('video');if(player){event.preventDefault();if(player.paused)void player.play().catch(()=>{});else player.pause();}}};
    createEffect(()=>{current();viewport();if(dialog.open)queueMicrotask(paint);});
    createEffect(()=>{if(props.open!==false)props.onSelection?.(current());});
    createEffect(()=>{
        if(props.open===false)return;
        untrack(()=>{
            closing=false;progress=0;panX=panY=dragX=dragY=0;axis='';pointers.clear();
            setSelectedId(props.attachment.id);setZoom(1);setStatus('');setLivePlaying(false);setViewport({width:innerWidth,height:innerHeight});
            if('requestIdleCallback' in window){const idle=requestIdleCallback(()=>setPreviewsReady(true));onCleanup(()=>cancelIdleCallback(idle));}
            if(reducedMotion()){progress=1;paint();}else settle();
            dialog.showModal();const stopChrome=attachViewerChrome(dialog);
            if(reducedMotion())dialog.animate([{opacity:0},{opacity:1}],{duration:150});
            dialog.addEventListener('wheel',wheel,{passive:false});window.addEventListener('resize',resize);
            onCleanup(()=>{stopChrome();animation?.cancel();for(const effect of dialog.getAnimations())effect.cancel();video?.pause();video=undefined;setPreviewsReady(false);previews.length=0;dialog.close();dialog.removeEventListener('wheel',wheel);window.removeEventListener('resize',resize);});
        });
    });
    return <dialog ref={dialog} class="media-viewer" data-chrome-visible="true" aria-label="Media viewer" data-media-zoom={zoom()} data-media-index={index()} data-media-count={items().length} data-media-loading={Boolean(props.loading)} data-media-message={text(current().data.message_id)} onCancel={event=>{event.preventDefault();close();}} onKeyDown={key} onPointerDown={down} onPointerMove={move} onPointerUp={up} onPointerCancel={up} onDblClick={event=>{if(!control(event))zoomTo(zoom()===1?2.5:1,event.clientX,event.clientY);}}>
        <div ref={backdrop} class="media-viewer-backdrop"/>
        <header class="media-viewer-toolbar" aria-label="Media controls" onPointerDown={event=>event.stopPropagation()}><button type="button" class="glass" aria-label="Close media viewer" title="Close" onClick={()=>close()}><svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><use width="24" height="24" href="/icons.svg#close"/></svg></button><div class="media-viewer-heading"><strong>{props.senderName||'Unknown sender'}</strong><time datetime={props.timestamp}>{props.subtitle}</time></div><div class="media-viewer-actions"><button type="button" class="glass" disabled={!navigator.share} aria-label="Share media" title="Share" onClick={()=>void share()}><svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><use width="24" height="24" href="/icons.svg#share"/></svg></button><a class="glass" href={mediaSource(current(),'original')} download={text(current().data.filename)} aria-label="Download original" title="Download Original"><svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><use width="24" height="24" href="/icons.svg#download"/></svg></a><button type="button" class="glass" disabled={!props.onShowInChat} aria-label="Show in Chat" title="Show in Chat" onClick={()=>{const attachment=current(),show=props.onShowInChat;close(()=>show?.(attachment));}}><svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><use width="24" height="24" href="/icons.svg#messages"/></svg></button></div></header>
        <For each={props.open!==false&&previewsReady()?[-1,1]:[]}>{(direction,position)=><div ref={element=>{previews[position()]=element;}} class="media-viewer-preview" data-direction={direction} inert aria-hidden="true"><Show when={items()[index()+direction]}>{attachment=><figure style={boxStyle(mediaBox(attachment()))}><MediaImage attachment={attachment()} variant={text(attachment().data.mime_type).startsWith('video/')?'poster':'display'} alt=""/></figure>}</Show></div>}</For>
        <figure ref={stage} class="media-viewer-stage" role="group" aria-label="Media content" tabindex="-1" autofocus style={boxStyle(box())}><Show keyed when={props.open!==false&&current().id}>{_id=><Show when={text(current().data.mime_type).startsWith('video/')} fallback={<><MediaImage attachment={current()} variant={text(current().data.mime_type)==='image/gif'?'original':'display'}/><Show when={fields(current().data.media).live}><button type="button" class="media-viewer-live" aria-label="Play Live Photo" onPointerDown={()=>setLivePlaying(true)} onPointerUp={()=>setLivePlaying(false)} onPointerCancel={()=>setLivePlaying(false)} onKeyDown={event=>{if(event.key===' '||event.key==='Enter'){event.preventDefault();setLivePlaying(true);}}} onKeyUp={event=>{if(event.key===' '||event.key==='Enter'){event.preventDefault();setLivePlaying(false);}}} onPointerLeave={()=>setLivePlaying(false)}><svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><use width="24" height="24" href="/icons.svg#media-live"/></svg>LIVE</button><Show when={livePlaying()}><VideoPlayer attachment={current()} live ref={value=>{video=value;value.muted=true;}}/></Show></Show></>}><VideoPlayer attachment={current()} ref={value=>{video=value;}}/></Show>}</Show></figure>
        <Show when={props.error}><p class="media-viewer-status" role="status">Attachment unavailable <button type="button" onClick={()=>props.onRetry?.()}>Try Again</button></p></Show>
        <Show when={status()}><p class="media-viewer-status" role="status">{status()}</p></Show>
        <button type="button" class="media-viewer-previous glass" data-nav-visible="false" aria-label="Previous media" title="Previous" disabled={index()<=0} onClick={()=>change(-1)}><svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><use width="24" height="24" href="/icons.svg#chevron"/></svg></button><button type="button" class="media-viewer-next glass" data-nav-visible="false" aria-label="Next media" title="Next" disabled={index()<0||index()>=items().length-1} onClick={()=>change(1)}><svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><use width="24" height="24" href="/icons.svg#chevron"/></svg></button>
    </dialog>;
}
