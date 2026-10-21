import { createSignal } from 'solid-js';
import { api, APIError } from './api';
import { parseSession, type DeviceSession } from '../data/types';
import { viewCache } from '../data/cache';
import { setPreferencePersistence } from '../app/preferences';
import { clearRouterPersistence, setRouterPersistence } from '../app/router';
import { LiveClient } from '../data/live';
import { strings as s } from '../app/strings';
const [session, setSession] = createSignal<DeviceSession | null>(), [error, setError] = createSignal(false), [live, setLive] = createSignal<LiveClient>();
const [notice,setNotice]=createSignal('');
const channel = typeof BroadcastChannel !== 'undefined' ? new BroadcastChannel('ab.session') : undefined;
let signingOut: Promise<void> | undefined;
let pwaSetup:Promise<void>=Promise.resolve();
async function clearPrivateData() {
    viewCache.disable();
    setPreferencePersistence(false);
    setRouterPersistence(false);
    clearRouterPersistence();
    await viewCache.wipe();
    await pwaSetup;
    if ('serviceWorker' in navigator) {
        for (const registration of await navigator.serviceWorker.getRegistrations()) {
            if(registration.active)await new Promise<void>(resolve=>{const channel=new MessageChannel();const timeout=setTimeout(resolve,5000);channel.port1.onmessage=()=>{clearTimeout(timeout);resolve();};registration.active!.postMessage({t:'wipe'},[channel.port2]);});
            await registration.unregister();
        }
    }
    if ('caches' in window)
        for (const key of await caches.keys())
            if (key.startsWith('ab.'))
                await caches.delete(key);
}
async function configurePWA(current: DeviceSession) {
    if (current.cache === 'memory') return;
    if (!('serviceWorker' in navigator))
        return;
    const registration = await navigator.serviceWorker.register('/sw.js', { scope: '/' });
    await navigator.serviceWorker.ready;
    if (session()?.device_id !== current.device_id) {
        await registration.unregister();
        return;
    }
    const worker = registration.active ?? registration.waiting;
    if (worker)
        await new Promise<void>(resolve => { 
const channel = new MessageChannel(); const timeout = setTimeout(resolve, 5000); channel.port1.onmessage = () => { clearTimeout(timeout); resolve(); }; worker.postMessage({ t: 'trusted' }, [channel.port2]); });
}
export async function establishSession(value: unknown) {
    if(signingOut) await signingOut.catch(()=>{});
    const current = parseSession(value);
    if (current.cache === 'persistent') {
        viewCache.enable(current.device_id);
        setPreferencePersistence(true);
        setRouterPersistence(true);
    }
    else
        await clearPrivateData();
    live()?.close();
    setLive(new LiveClient(() => { void endSession(false, true); },frame=>{if(frame.t==='signin')setNotice(s.newSignIn);}));
    setSession(current);
    setError(false);
    pwaSetup=configurePWA(current).catch(() => { });
}
export async function loadSession() {
    try {
        await establishSession(await api('session'));
    }
    catch (error) {
        if (error instanceof APIError && error.status === 401) {
            await clearPrivateData();
            setSession(null);
        }
        else
            setError(true);
    }
}
export function endSession(remote = true, broadcast = true) {
    if (signingOut)
        return signingOut;
    const current = session();
    live()?.close();
    setLive(undefined);
    setSession(null);
    setNotice('');
    signingOut = (async () => {
        if (broadcast)
            channel?.postMessage({ t: 'signed-out', device: current?.device_id });
        const clearing=clearPrivateData();
        try {
            if (remote)
                await api('logout', {});
        }
        finally {
            await clearing;
        }
    })().finally(() => { signingOut = undefined; });
    return signingOut;
}
channel?.addEventListener('message', event => { if (event.data?.t === 'signed-out' && event.data.device === session()?.device_id)
    void endSession(false, false); });
window.addEventListener('pagehide', () => { live()?.close(); });
let lastInput = Date.now();
for (const event of ['pointerdown', 'keydown', 'input'])
    window.addEventListener(event, () => { lastInput = Date.now(); }, { passive: true });
setInterval(() => { if (session()?.cache === 'memory' && Date.now() - lastInput >= 30 * 60 * 1000)
    void endSession(); }, 15000);
window.addEventListener('pageshow',event=>{if(event.persisted){if(session()?.cache==='memory'&&Date.now()-lastInput>=30*60*1000)void endSession();else void loadSession();}});
export const auth = { session, error, live, notice, dismissNotice:()=>setNotice(''), retry: loadSession };
