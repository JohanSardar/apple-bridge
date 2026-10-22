import {crossfade,reducedMotion,springAnimation} from './animate';
import {sampleSpring} from './spring';
export function receiveBubble(element:HTMLElement,sent:boolean){
    element.dataset.motion='receive';
    element.style.transformOrigin=sent?'right bottom':'left bottom';
    const reset=()=>{element.style.transform='';element.style.transformOrigin='';delete element.dataset.motion;};
    if(reducedMotion()){
        const animation=crossfade(element);animation.onfinish=reset;
        return ()=>{animation.cancel();reset();};
    }
    const animation=springAnimation(value=>{element.style.transform=`scale(${value})`;},.92,1,{response:.38,damping:.75},0,reset);
    return ()=>{animation.cancel();reset();};
}

function pop(element:HTMLElement,name:string,from:number){
    element.dataset.motion=name;
    const reset=()=>{element.style.transform='';delete element.dataset.motion;};
    if(reducedMotion()){const animation=crossfade(element);animation.onfinish=reset;return ()=>{animation.cancel();reset();};}
    const animation=springAnimation(value=>{element.style.transform=`scale(${value})`;},from,1,{response:.3,damping:.65},0,reset);
    return ()=>{animation.cancel();reset();};
}
export const popReaction=(element:HTMLElement)=>pop(element,'reaction-pop',.6);
export const enterNewPill=(element:HTMLElement)=>pop(element,'new-pill',.9);
export function enterThreadRow(element:HTMLElement,index:number){
    element.dataset.motion='thread-enter';
    const frames=reducedMotion()?[{opacity:0},{opacity:1}]:Array.from({length:31},(_,frame)=>{const y=frame===30?0:sampleSpring(8,0,0,frame*.015,{response:.35,damping:.85}).value;return {transform:`translateY(${y}px)`,opacity:1-Math.min(1,Math.abs(y/8))};});
    const animation=element.animate(frames,{duration:reducedMotion()?150:450,delay:reducedMotion()?0:index*20,fill:'backwards',easing:'linear'});
    animation.onfinish=()=>{delete element.dataset.motion;};
    return ()=>{animation.cancel();delete element.dataset.motion;};
}
export function highlightMessage(element:HTMLElement){
    element.dataset.motion='message-highlight';
    const animation=reducedMotion()?crossfade(element):element.animate([{filter:'drop-shadow(0 0 0 transparent)'},{filter:'drop-shadow(0 0 5px var(--blue))'},{filter:'drop-shadow(0 0 0 transparent)'}],{duration:900,easing:'ease-in-out'});
    animation.onfinish=()=>{delete element.dataset.motion;};
    return ()=>{animation.cancel();delete element.dataset.motion;};
}
export function revealHistory(outer:HTMLElement,inner:HTMLElement,closed:()=>void){
    let open=false,natural=0,current=0,motion:ReturnType<typeof springAnimation>|undefined,fade:Animation|undefined;
    outer.style.height='0px';outer.style.overflow='hidden';
    const cancel=()=>{motion?.cancel();fade?.cancel();};
    const move=()=>{
        cancel();if(open&&!natural)return;
        outer.dataset.motion='edit-expand';
        if(reducedMotion()){
            outer.style.height=open?'':natural+'px';outer.style.overflow='';
            fade=crossfade(outer,open);fade.onfinish=()=>{delete outer.dataset.motion;if(!open){current=0;outer.style.height='0px';closed();}};return;
        }
        outer.style.overflow='hidden';
        motion=springAnimation(value=>{current=Math.max(0,value);outer.style.height=current+'px';},current,open?natural:0,{response:.35,damping:1},0,()=>{delete outer.dataset.motion;if(open){outer.style.height='';outer.style.overflow='';current=natural;}else closed();});
    };
    const observer=new ResizeObserver(entries=>{const next=entries[0].borderBoxSize?.[0]?.blockSize??entries[0].contentRect.height;if(next===natural)return;natural=next;if(open){if(outer.style.height)move();else current=natural;}});observer.observe(inner);
    return {setOpen(value:boolean){if(value===open)return;if(!outer.style.height)current=natural;open=value;move();},dispose(){cancel();observer.disconnect();}};
}
