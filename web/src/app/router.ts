import { createSignal } from 'solid-js';
export type Route = { path:string; chat?:string; message?:string; section?:string };
function parse(path:string):Route {
  const parts=path.split('/').filter(Boolean).map(part=>{try{return decodeURIComponent(part);}catch{return part;}});
  return {path,chat:parts[0]==='messages'?parts[1]:undefined,message:parts[0]==='messages'?parts[2]:undefined,section:parts[0]==='settings'?parts[1]??'devices':undefined};
}
const [route,setRoute]=createSignal(parse(location.pathname));
let key=history.state?.abKey??crypto.randomUUID();
let persistent=false;
export function setRouterPersistence(value:boolean){persistent=value;}
const positions=new Map<string,Record<string,number>>();
history.replaceState({...history.state,abKey:key},'');
history.scrollRestoration='manual';
function save(){const offsets:Record<string,number>={};document.querySelectorAll<HTMLElement>('[data-scroll]').forEach(element=>{offsets[element.dataset.scroll!]=element.scrollTop;});positions.set(key,offsets);}
function restore(){const offsets=positions.get(key)??{};requestAnimationFrame(()=>{document.querySelectorAll<HTMLElement>('[data-scroll]').forEach(element=>{element.scrollTop=offsets[element.dataset.scroll!]??0;});});}
window.addEventListener('popstate',()=>{save();key=history.state?.abKey??crypto.randomUUID();setRoute(parse(location.pathname));restore();});
window.addEventListener('pagehide',()=>{if(!persistent)return;save();try{sessionStorage.setItem('ab.scroll',JSON.stringify({key,offsets:positions.get(key)}));}catch{return;}});
try{const saved=JSON.parse(sessionStorage.getItem('ab.scroll')??'null');if(saved?.key===key){positions.set(key,saved.offsets);restore();}}catch{positions.delete(key);}
export function clearRouterPersistence(){positions.clear();try{sessionStorage.removeItem('ab.scroll');}catch{return;}}
export const router={route,navigate(path:string,replace=false){save();key=crypto.randomUUID();history[replace?'replaceState':'pushState']({abKey:key},'',path);setRoute(parse(path));restore();},back(){if(history.length>1)history.back();else this.navigate('/messages',true);}};
