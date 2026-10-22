const family=(value:string)=>value.trim().replace(/^['"]|['"]$/gu,'').toLowerCase();
export function createFontWatcher(changed:()=>void){
    const promises=new WeakSet<Promise<unknown>>();let callback:(()=>void)|undefined=changed;
    const notify=()=>callback?.();
    const follow=(promise:Promise<unknown>)=>{if(!promises.has(promise)){promises.add(promise);void promise.then(notify,()=>undefined);}};
    let observedSize:string|undefined;
    const observer=new ResizeObserver(entries=>{
        const bounds=entries[0]?.contentRect;if(!bounds)return;
        const size=bounds.width+':'+bounds.height;
        if(observedSize!==undefined&&observedSize!==size)notify();
        observedSize=size;
    });let observed:HTMLElement|undefined;
    function observe(probe:HTMLElement){
        if(probe!==observed){observer.disconnect();observer.observe(probe);observed=probe;observedSize=undefined;}
    }
    function watch(element:HTMLElement,probe:HTMLElement){
        observe(probe);
        if(document.fonts.status==='loading')follow(document.fonts.ready);
        const style=getComputedStyle(element),families=new Set(style.fontFamily.split(',').map(family)),weight=Number(style.fontWeight)||400;
        for(const face of document.fonts){
            if(!families.has(family(face.family)))continue;
            const weights=face.weight.split(/\s+/u).map(value=>value==='normal'?400:value==='bold'?700:Number(value));
            if(Number.isFinite(weights[0])&&(weight<weights[0]||weight>(weights[1]??weights[0])))continue;
            if(face.status!=='loaded')follow(face.loaded);
        }
    }
    return {observe,watch,dispose:()=>{callback=undefined;observed=undefined;observer.disconnect();}};
}
