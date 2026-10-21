import type { Cursor, ViewData } from './types';
export type CachedView={key:string;owner:string;data:ViewData;cursor?:Cursor;at:number};
let database:IDBDatabase|undefined, opening:Promise<IDBDatabase>|undefined, owner:string|undefined,generation=0;
let writes:Promise<unknown>=Promise.resolve();
function open(){
  if(database)return Promise.resolve(database);
  if(opening)return opening;
  opening=new Promise<IDBDatabase>((resolve,reject)=>{const request=indexedDB.open('ab.views',1);request.onupgradeneeded=()=>request.result.createObjectStore('views',{keyPath:'key'});request.onerror=()=>reject(request.error);request.onsuccess=()=>{database=request.result;database.onversionchange=()=>{database?.close();database=undefined;opening=undefined;};resolve(database);};});
  return opening;
}
function request<T>(value:IDBRequest<T>){return new Promise<T>((resolve,reject)=>{value.onsuccess=()=>resolve(value.result);value.onerror=()=>reject(value.error);});}
export const viewCache={
  enable(device:string){owner=device;generation++;},
  disable(){owner=undefined;generation++;},
  async read(key:string){const device=owner,version=generation;if(!device)return;const db=await open();if(version!==generation)return;const value=await request(db.transaction('views').objectStore('views').get(key)) as CachedView|undefined;return owner===device&&value?.owner===device?value:undefined;},
  write(key:string,data:ViewData,cursor?:Cursor){const device=owner,version=generation;if(!device)return Promise.resolve();writes=writes.catch(()=>{}).then(async()=>{if(version!==generation)return;const db=await open();if(version!==generation)return;await new Promise<void>((resolve,reject)=>{const transaction=db.transaction('views','readwrite'),store=transaction.objectStore('views');store.put({key,owner:device,data,cursor,at:Date.now()});const all=store.getAll();all.onsuccess=()=>{const values=all.result as CachedView[];values.sort((a,b)=>b.at-a.at);for(const value of values.slice(100))store.delete(value.key);};transaction.oncomplete=()=>resolve();transaction.onerror=()=>reject(transaction.error);});});return writes;},
  async wipe(){this.disable();await writes.catch(()=>{});if(opening)await opening.catch(()=>{});database?.close();database=undefined;opening=undefined;await new Promise<void>((resolve,reject)=>{const deletion=indexedDB.deleteDatabase('ab.views');deletion.onsuccess=()=>resolve();deletion.onerror=()=>reject(deletion.error);deletion.onblocked=()=>reject(new Error('Cache deletion blocked'));});}
};
