import type { DeviceSession } from '../../data/types';
const prefix = 'ab.messages.draft.', memory = new Map<string, string>(), pending = new Map<string, ReturnType<typeof setTimeout>>();
let forward:{deviceID:string;draftID:string}|undefined;
const key = (session: DeviceSession, id: string) => prefix + encodeURIComponent(session.device_id) + '.' + encodeURIComponent(id);
function flush(name: string) {
    if (!pending.has(name)) return;
    clearTimeout(pending.get(name)); pending.delete(name);
    const text = memory.get(name) ?? '';
    try { if (text) localStorage.setItem(name, text); else localStorage.removeItem(name); }
    catch { }
}
export function readDraft(session: DeviceSession, id: string) {
    const name = key(session, id);
    if (memory.has(name)) return memory.get(name)!;
    if (session.cache === 'persistent'&&!id.startsWith('forward:')) {
        try { return localStorage.getItem(name) ?? ''; }
        catch { }
    }
    return '';
}
export function writeDraft(session: DeviceSession, id: string, text: string) {
    const name = key(session, id);
    memory.set(name, text);
    if (session.cache === 'persistent'&&!id.startsWith('forward:')) {
        clearTimeout(pending.get(name));
        pending.set(name, setTimeout(() => flush(name), 300));
    }
}
export function flushDraft(session: DeviceSession, id: string) { flush(key(session, id)); }
export function discardDraft(session:DeviceSession,id:string){const name=key(session,id);clearTimeout(pending.get(name));pending.delete(name);memory.delete(name);try{localStorage.removeItem(name);}catch{}}
export function prepareForward(session:DeviceSession,text:string){if(forward)discardDraft(session,forward.draftID);const draftID='forward:'+crypto.randomUUID();writeDraft(session,draftID,text);forward={deviceID:session.device_id,draftID};return draftID;}
export function takeForward(session:DeviceSession){const value=forward;forward=undefined;return value?.deviceID===session.device_id?value:undefined;}
export function clearDrafts() {
    forward=undefined;
    for (const timer of pending.values()) clearTimeout(timer);
    pending.clear(); memory.clear();
    try { for (const name of Object.keys(localStorage)) if (name.startsWith(prefix)) localStorage.removeItem(name); }
    catch { }
}
const flushAll = () => { for (const name of pending.keys()) flush(name); };
window.addEventListener('pagehide', flushAll);
document.addEventListener('visibilitychange', flushAll);
