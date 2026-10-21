import type { DeviceSession } from '../../data/types';
const prefix = 'ab.messages.draft.', memory = new Map<string, string>();
const key = (session: DeviceSession, id: string) => prefix + encodeURIComponent(session.device_id) + '.' + encodeURIComponent(id);
export function readDraft(session: DeviceSession, id: string) {
    const name = key(session, id);
    if (session.cache === 'persistent') {
        try { return localStorage.getItem(name) ?? memory.get(name) ?? ''; }
        catch { }
    }
    return memory.get(name) ?? '';
}
export function writeDraft(session: DeviceSession, id: string, text: string) {
    const name = key(session, id);
    if (text) memory.set(name, text); else memory.delete(name);
    if (session.cache === 'persistent') {
        try { if (text) localStorage.setItem(name, text); else localStorage.removeItem(name); }
        catch { }
    }
}
export function clearDrafts() {
    memory.clear();
    try { for (const name of Object.keys(localStorage)) if (name.startsWith(prefix)) localStorage.removeItem(name); }
    catch { }
}
