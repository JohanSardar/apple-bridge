import { createEffect, createMemo, createSignal, onCleanup, type Accessor } from 'solid-js';
import type { BridgeRecord } from '../data/types';
import { blobURL, object, text } from './store';
type Body = { plain_text: string; html: string; blocks: unknown[] };
type Loaded = { key: string; body?: Body; unavailable?: boolean };
export const externalBody = (data: Record<string, unknown>) => data.body_status === 'external' || Boolean(data.body_blob);
async function readBody(response: Response, size: number, digest: string): Promise<Body> {
    if (!response.ok || !response.body) throw Error('Body unavailable');
    const reader = response.body.getReader(), chunks: Uint8Array[] = [];
    let received = 0;
    try {
        for (;;) {
            const { done, value } = await reader.read();
            if (done) break;
            received += value.byteLength;
            if (received > size) throw Error('Body size mismatch');
            chunks.push(value);
        }
    } finally { await reader.cancel().catch(() => {}); reader.releaseLock(); }
    if (received !== size) throw Error('Body size mismatch');
    const bytes = new Uint8Array(received);
    let offset = 0;
    for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
    const actual = [...new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))].map(byte => byte.toString(16).padStart(2, '0')).join('');
    if (actual !== digest) throw Error('Body digest mismatch');
    const body = object(JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes)));
    if (typeof body.plain_text !== 'string' || typeof body.html !== 'string' || !Array.isArray(body.blocks) || !body.blocks.every(block => typeof object(block).type === 'string')) throw Error('Invalid body');
    return { plain_text: body.plain_text, html: body.html, blocks: body.blocks };
}
export function useNoteBody(record: Accessor<BridgeRecord>) {
    const [loaded, setLoaded] = createSignal<Loaded>();
    const [attempt, setAttempt] = createSignal(0);
    const key = createMemo(() => {
        const value = record();
        return JSON.stringify([value.id, value.version, Boolean(value.data.locked), externalBody(value.data), value.data.body_blob]);
    });
    createEffect(() => {
        const current = key(); attempt();
        const [id, , locked, external, raw] = JSON.parse(current) as [string, number, boolean, boolean, unknown];
        setLoaded(undefined);
        if (locked || !external) return;
        const blob = object(raw), digest = text(blob.digest), size = Number(blob.size), url = blobURL(digest, 'original', id);
        if (!url || blob.state !== 'present' || !Number.isSafeInteger(size) || size <= 0) { setLoaded({ key: current, unavailable: true }); return; }
        const controller = new AbortController();
        let disposed = false;
        onCleanup(() => { disposed = true; controller.abort(); });
        void fetch(url, { credentials: 'same-origin', cache: 'no-store', signal: AbortSignal.any([controller.signal, AbortSignal.timeout(10000)]) })
            .then(response => readBody(response, size, digest))
            .then(body => { if (!disposed) setLoaded({ key: current, body }); })
            .catch(() => { if (!disposed) setLoaded({ key: current, unavailable: true }); });
    });
    const current = () => loaded()?.key === key() ? loaded() : undefined;
    const status = () => !externalBody(record().data) || record().data.locked ? 'inline' : current()?.body ? 'ready' : current()?.unavailable ? 'unavailable' : 'loading';
    const data = () => status() === 'ready' ? { ...record().data, ...current()!.body } : externalBody(record().data) ? { ...record().data, plain_text: '', html: '', blocks: [] } : record().data;
    return { status, data, retry: () => setAttempt(value => value + 1) };
}
