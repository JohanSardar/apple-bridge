import {reducedMotion,springAnimation} from '../../motion/animate';
export function attachViewerChrome(root:HTMLDialogElement){
    const desktop=matchMedia('(min-width:700px) and (pointer:fine) and (hover:hover)'),motion=matchMedia('(prefers-reduced-motion:reduce)');
    const previous=root.querySelector<HTMLButtonElement>('.media-viewer-previous')!,next=root.querySelector<HTMLButtonElement>('.media-viewer-next')!;
    type Track={value:number;target:number;animation?:ReturnType<typeof springAnimation>};
    const chrome:Track={value:1,target:1},left:Track={value:0,target:0},right:Track={value:0,target:0};
    let visible=true,stopped=false,timer:ReturnType<typeof setTimeout>,last={x:-1,y:-1},press:{id:number;x:number;y:number;at:number;visible:boolean;control:boolean;moved:boolean}|undefined;
    const writeChrome=(value:number)=>root.style.setProperty('--media-chrome-opacity',String(value));
    const writeEdge=(button:HTMLButtonElement,value:number,direction:number)=>{button.style.setProperty('--media-edge-opacity',String(value));button.style.transform=reducedMotion()?'':`translate3d(${direction*(1-value)*8}px,0,0) scale(${.94+.06*value})`;};
    const animate=(track:Track,target:number,write:(value:number)=>void)=>{
        if(track.target===target&&track.value===target)return;
        if(track.target===target&&track.animation)return;
        track.animation?.cancel();track.target=target;
        if(reducedMotion()){track.value=target;write(target);track.animation=undefined;return;}
        track.animation=springAnimation(value=>{if(stopped)return;track.value=value;write(value);},track.value,target,{response:.3,damping:1},0,()=>{track.animation=undefined;});
    };
    const edge=(button:HTMLButtonElement,track:Track,show:boolean,direction:number)=>{const target=show&&!button.disabled?1:0;button.dataset.navVisible=String(Boolean(target));animate(track,target,value=>writeEdge(button,value,direction));};
    const updateEdges=()=>{edge(previous,left,desktop.matches&&(last.x<80||document.activeElement===previous),-1);edge(next,right,desktop.matches&&(last.x>innerWidth-80||document.activeElement===next),1);};
    const show=(value:boolean)=>{visible=value;root.dataset.chromeVisible=String(value);animate(chrome,value?1:0,writeChrome);};
    const arm=()=>{clearTimeout(timer);if(!visible||press||stopped)return;timer=setTimeout(()=>{const active=document.activeElement;if(active instanceof HTMLElement&&active.matches(':focus-visible')&&active.closest('button,a,input'))arm();else show(false);},2000);};
    const wake=()=>{show(true);arm();};
    const down=(event:PointerEvent)=>{clearTimeout(timer);press={id:event.pointerId,x:event.clientX,y:event.clientY,at:event.timeStamp,visible,moved:false,control:event.target instanceof Element&&Boolean(event.target.closest('button,a,input'))};};
    const move=(event:PointerEvent)=>{if(press){press.moved||=Math.hypot(event.clientX-press.x,event.clientY-press.y)>6;clearTimeout(timer);return;}if(event.pointerType!=='mouse'||event.clientX===last.x&&event.clientY===last.y)return;last={x:event.clientX,y:event.clientY};wake();updateEdges();};
    const up=(event:PointerEvent)=>{if(!press||press.id!==event.pointerId)return;const start=press;press=undefined;if(event.type!=='pointercancel'&&!start.control&&!start.moved&&Math.hypot(event.clientX-start.x,event.clientY-start.y)<6&&event.timeStamp-start.at<500)show(!start.visible);arm();};
    const leave=()=>{last={x:innerWidth/2,y:innerHeight/2};updateEdges();};
    const focus=()=>{wake();updateEdges();};
    const changed=()=>{for(const track of [chrome,left,right]){track.animation?.cancel();track.animation=undefined;}chrome.value=visible?1:0;writeChrome(chrome.value);left.value=left.target;right.value=right.target;writeEdge(previous,left.value,-1);writeEdge(next,right.value,1);updateEdges();};
    writeChrome(1);writeEdge(previous,0,-1);writeEdge(next,0,1);previous.dataset.navVisible='false';next.dataset.navVisible='false';root.dataset.chromeVisible='true';arm();
    root.addEventListener('pointerdown',down,true);root.addEventListener('pointermove',move);root.addEventListener('pointerup',up,true);root.addEventListener('pointercancel',up,true);root.addEventListener('pointerleave',leave);root.addEventListener('keydown',wake);root.addEventListener('focusin',focus);root.addEventListener('focusout',updateEdges);desktop.addEventListener('change',changed);motion.addEventListener('change',changed);
    return ()=>{stopped=true;clearTimeout(timer);for(const track of [chrome,left,right])track.animation?.cancel();root.removeEventListener('pointerdown',down,true);root.removeEventListener('pointermove',move);root.removeEventListener('pointerup',up,true);root.removeEventListener('pointercancel',up,true);root.removeEventListener('pointerleave',leave);root.removeEventListener('keydown',wake);root.removeEventListener('focusin',focus);root.removeEventListener('focusout',updateEdges);desktop.removeEventListener('change',changed);motion.removeEventListener('change',changed);};
}
