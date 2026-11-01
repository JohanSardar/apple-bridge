const family=(value:string)=>value.trim().replace(/^['"]|['"]$/gu,'').toLowerCase();
export function matchingFonts(value:string){const families=new Set(value.split(',').map(family));return [...document.fonts].filter(face=>families.has(family(face.family)));}
export function createFontWatcher(changed:()=>void){
    const promises=new WeakSet<Promise<unknown>>();let callback:(()=>void)|undefined=changed;
    const notify=()=>callback?.();
    document.fonts.addEventListener('loadingdone',notify);
    const follow=(promise:Promise<unknown>)=>{if(!promises.has(promise)){promises.add(promise);void promise.then(notify,()=>undefined);}};
    let observedSize:string|undefined;
    const resized=(bounds:DOMRectReadOnly)=>{
        const size=bounds.width+':'+bounds.height;
        if(observedSize!==undefined&&observedSize!==size)notify();
        observedSize=size;
    };
    function watch(style:CSSStyleDeclaration){
        if(document.fonts.status==='loading')follow(document.fonts.ready);
        for(const face of matchingFonts(style.fontFamily))if(face.status!=='loaded')follow(face.loaded);
    }
    return {resized,watch,dispose:()=>{callback=undefined;document.fonts.removeEventListener('loadingdone',notify);}};
}
