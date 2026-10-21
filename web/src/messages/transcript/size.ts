const callbacks=new Map<Element,(width:number,height:number)=>void>();
let observer:ResizeObserver|undefined;
export function observeBubbleSize(element:HTMLElement,callback:(width:number,height:number)=>void){
    observer??=new ResizeObserver(entries=>{for(const entry of entries){const box=entry.borderBoxSize?.[0],rect=box?undefined:entry.target.getBoundingClientRect();callbacks.get(entry.target)?.(box?.inlineSize??rect!.width,box?.blockSize??rect!.height);}});
    callbacks.set(element,callback);observer.observe(element);
    return()=>{callbacks.delete(element);observer?.unobserve(element);if(!callbacks.size){observer?.disconnect();observer=undefined;}};
}
