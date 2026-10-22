import { createSignal } from 'solid-js';
export type Route = {
    path: string;
    chat?: string;
    message?: string;
    panel?: 'details';
    section?: string;
};
function parse(path: string): Route {
    const parts = new URL(path,location.origin).pathname.split('/').filter(Boolean).map(part => { try {
        return decodeURIComponent(part);
    }
    catch {
        return part;
    } });
    return { path, chat: parts[0] === 'messages' ? parts[1] : undefined, message: parts[0] === 'messages'&&parts[2]!=='details' ? parts[2] : undefined, panel:parts[0]==='messages'&&parts[2]==='details'?'details':undefined, section: parts[0] === 'settings' ? parts[1] ?? 'devices' : undefined };
}
const [route, setRoute] = createSignal(parse(location.pathname+location.search));
let key = history.state?.abKey ?? crypto.randomUUID();
let persistent = false;
export function setRouterPersistence(value: boolean) { persistent = value; }
const positions = new Map<string, Record<string, number>>();
history.replaceState({ ...history.state, abKey: key }, '');
history.scrollRestoration = 'manual';
function save() { const offsets: Record<string, number> = {}; document.querySelectorAll<HTMLElement>('[data-scroll]').forEach(element => { offsets[element.dataset.scroll!] = element.scrollTop; }); positions.set(key, offsets); }
let restoreObserver:MutationObserver|undefined;
let restoreFrame=0;
let restoreTimeout:ReturnType<typeof setTimeout>|undefined;
function stopRestore(){restoreObserver?.disconnect();restoreObserver=undefined;cancelAnimationFrame(restoreFrame);clearTimeout(restoreTimeout);}
function restore(){
    stopRestore();
    const offsets=positions.get(key);
    if(!offsets)return;
    const pending=new Set(Object.keys(offsets));
    const stable=new Map<string,number>();
    const attempt=()=>{
        for(const element of document.querySelectorAll<HTMLElement>('[data-scroll]')){
            const name=element.dataset.scroll!;
            if(!pending.has(name))continue;
            const before=element.scrollTop;
            element.scrollTop=offsets[name];
            if(Math.abs(element.scrollTop-offsets[name])<1){const count=Math.abs(before-offsets[name])<1?(stable.get(name)??0)+1:0;stable.set(name,count);if(count>=2)pending.delete(name);else{cancelAnimationFrame(restoreFrame);restoreFrame=requestAnimationFrame(attempt);}}
        }
        if(!pending.size)stopRestore();
    };
    restoreObserver=new MutationObserver(()=>{cancelAnimationFrame(restoreFrame);restoreFrame=requestAnimationFrame(attempt);});
    restoreObserver.observe(document.documentElement,{childList:true,subtree:true,attributes:true,attributeFilter:['style']});
    restoreFrame=requestAnimationFrame(attempt);
    restoreTimeout=setTimeout(stopRestore,10000);
}
for(const type of ['pointerdown','keydown','wheel','touchstart'])window.addEventListener(type,event=>{if(event.isTrusted)stopRestore();},{capture:true,passive:true});
window.addEventListener('popstate', () => { save(); key = history.state?.abKey ?? crypto.randomUUID(); setRoute(parse(location.pathname+location.search)); restore(); });
window.addEventListener('pagehide', () => { if (!persistent)
    return; save(); try {
    sessionStorage.setItem('ab.scroll', JSON.stringify({ key, offsets: positions.get(key) }));
}
catch {
    return;
} });
try {
    const saved = JSON.parse(sessionStorage.getItem('ab.scroll') ?? 'null');
    if (saved?.key === key) {
        positions.set(key, saved.offsets);
        restore();
    }
}
catch {
    positions.delete(key);
}
export function clearRouterPersistence() { stopRestore(); positions.clear(); try {
    sessionStorage.removeItem('ab.scroll');
}
catch {
    return;
} }
export const router = { route, navigate(path: string, replace = false) { save(); key = crypto.randomUUID(); history[replace ? 'replaceState' : 'pushState']({ abKey: key }, '', path); setRoute(parse(path)); restore(); }, back() { if (history.length > 1)
        history.back();
    else
        this.navigate('/messages', true); } };
