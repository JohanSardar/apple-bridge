import type { DeviceSession } from '../../data/types';
const prefix = 'ab.messages.draft.', memory = new Map<string, string>(), pending = new Map<string, ReturnType<typeof setTimeout>>();
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
    if (session.cache === 'persistent') {
        try { return localStorage.getItem(name) ?? ''; }
        catch { }
    }
    return '';
}
export function writeDraft(session: DeviceSession, id: string, text: string) {
    const name = key(session, id);
    memory.set(name, text);
    if (session.cache === 'persistent') {
        clearTimeout(pending.get(name));
        pending.set(name, setTimeout(() => flush(name), 300));
    }
}
export function flushDraft(session: DeviceSession, id: string) { flush(key(session, id)); }
export function clearDrafts() {
    for (const timer of pending.values()) clearTimeout(timer);
    pending.clear(); memory.clear();
    try { for (const name of Object.keys(localStorage)) if (name.startsWith(prefix)) localStorage.removeItem(name); }
    catch { }
}
const flushAll = () => { for (const name of pending.keys()) flush(name); };
window.addEventListener('pagehide', flushAll);
document.addEventListener('visibilitychange', flushAll);
