import { createEffect, onCleanup } from 'solid-js';
import { crossfade, reducedMotion, springAnimation } from '../../motion/animate';
export function createReorderMotion(root:()=>HTMLElement|undefined,items:()=>readonly unknown[]) {
    const animations=new Map<string,ReturnType<typeof springAnimation>>();
    let frame=0,before:Map<string,number>|undefined;
    createEffect(()=>{
        items();const container=root();if(!container)return;
        before??=new Map(Array.from(container.querySelectorAll<HTMLElement>('.conversation-row'),row=>[row.dataset.chat!,row.getBoundingClientRect().top]));
        const previous=before;
        for(const animation of animations.values())animation.cancel();
        cancelAnimationFrame(frame);
        frame=requestAnimationFrame(()=>{
            frame=0;before=undefined;
            const visible=new Set<string>();
            for(const row of container.querySelectorAll<HTMLElement>('.conversation-row')){
                const key=row.dataset.chat!,current=animations.get(key),velocity=current?.state.velocity??0;
                visible.add(key);current?.cancel();animations.delete(key);row.style.transform='';
                const target=row.getBoundingClientRect().top,offset=previous.has(key)?previous.get(key)!-target:0;
                if(Math.abs(offset)<1)continue;
                if(reducedMotion()){crossfade(row);continue;}
                animations.set(key,springAnimation(value=>{row.style.transform=`translateY(${value}px)`;},offset,0,{response:.38,damping:.86},velocity,()=>{row.style.transform='';animations.delete(key);}));
            }
            for(const key of animations.keys())if(!visible.has(key))animations.delete(key);
        });
    });
    onCleanup(()=>{cancelAnimationFrame(frame);for(const animation of animations.values())animation.cancel();});
}
