import type { Cursor, ViewData } from './types';
export type CachedView = {
    key: string;
    owner: string;
    data: ViewData;
    cursor?: Cursor;
    at: number;
};
let database: IDBDatabase | undefined, opening: Promise<IDBDatabase> | undefined, owner: string | undefined, generation = 0;
let writes: Promise<unknown> = Promise.resolve(), presentation: Promise<void> | undefined;
const pending = new Map<string, { data: ViewData; cursor?: Cursor; version: number; promise: Promise<unknown> }>();
const memory = new Map<string, CachedView>();
function remember(value: CachedView) {
    memory.delete(value.key); memory.set(value.key, value);
    while (memory.size > 32) memory.delete(memory.keys().next().value!);
    return value;
}
function afterPaint() {
    return presentation??=new Promise<void>(resolve=>{
        let frame=0,finished=false;
        const finish=()=>{if(finished)return;finished=true;cancelAnimationFrame(frame);document.removeEventListener('visibilitychange',hidden);presentation=undefined;resolve();};
        const hidden=()=>{if(document.hidden)finish();};
        if(document.hidden){setTimeout(finish,0);return;}
        document.addEventListener('visibilitychange',hidden);
        frame=requestAnimationFrame(()=>setTimeout(finish,0));
    });
}
function open() {
    if (database)
        return Promise.resolve(database);
    if (opening)
        return opening;
    opening = new Promise<IDBDatabase>((resolve, reject) => { const request = indexedDB.open('ab.views', 3); request.onupgradeneeded = () => {
        const store=request.result.objectStoreNames.contains('views')?request.transaction!.objectStore('views'):request.result.createObjectStore('views',{keyPath:'key'});
        if(!store.indexNames.contains('at'))store.createIndex('at','at');
        const cursor=store.openKeyCursor();cursor.onsuccess=()=>{const entry=cursor.result;if(!entry)return;try{if(JSON.parse(String(entry.key))[0]==='chats')store.delete(entry.primaryKey);}catch{}entry.continue();};
    }; request.onerror = () => reject(request.error); request.onsuccess = () => { database = request.result; database.onversionchange = () => { database?.close(); database = undefined; opening = undefined; }; resolve(database); }; });
    return opening;
}
function request<T>(value: IDBRequest<T>) { return new Promise<T>((resolve, reject) => { value.onsuccess = () => resolve(value.result); value.onerror = () => reject(value.error); }); }
export const viewCache = {
    enable(device: string) { owner = device; generation++; memory.clear(); },
    disable() { owner = undefined; generation++; memory.clear(); },
    async read(key: string) {
        const device = owner, version = generation;
        if (!device) return;
        const cached = memory.get(key);
        if (cached) return remember(cached);
        const db = await open();
        if (version !== generation) return;
        const value = await request(db.transaction('views').objectStore('views').get(key)) as CachedView | undefined;
        if (version !== generation || owner !== device) return;
        const current = memory.get(key);
        if (current) return remember(current);
        return value?.owner === device ? remember(value) : undefined;
    },
    write(key: string, data: ViewData, cursor?: Cursor) {
        const device = owner, version = generation;
        if (!device) return Promise.resolve();
        remember({ key, owner: device, data, cursor, at: Date.now() });
        const queued = pending.get(key);
        if (queued?.version === version) {
            queued.data = data; queued.cursor = cursor;
            return queued.promise;
        }
        const entry = { data, cursor, version, promise: Promise.resolve() as Promise<unknown> }, painted = afterPaint();
        pending.set(key, entry);
        entry.promise = writes.catch(() => {}).then(async () => {
            await painted;
            if (version !== generation) return;
            const db = await open();
            if (version !== generation) return;
            if (pending.get(key) === entry) pending.delete(key);
            await new Promise<void>((resolve, reject) => {
                const transaction = db.transaction('views', 'readwrite'), store = transaction.objectStore('views');
                store.put({ key, owner: device, data: entry.data, cursor: entry.cursor, at: Date.now() });
                const keys = store.index('at').getAllKeys();
                keys.onsuccess = () => { for (const key of keys.result.slice(0,-100)) store.delete(key); };
                transaction.oncomplete = () => resolve(); transaction.onerror = () => reject(transaction.error);
            });
        }).finally(() => { if (pending.get(key) === entry) pending.delete(key); });
        writes = entry.promise;
        return writes;
    },
    async wipe() { this.disable(); await writes.catch(() => { }); if (opening)
        await opening.catch(() => { }); database?.close(); database = undefined; opening = undefined; await new Promise<void>((resolve, reject) => { const deletion = indexedDB.deleteDatabase('ab.views'); deletion.onsuccess = () => resolve(); deletion.onerror = () => reject(deletion.error); deletion.onblocked = () => reject(new Error('Cache deletion blocked')); }); }
};
