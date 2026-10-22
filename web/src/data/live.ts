import { createSignal, untrack } from 'solid-js';
import { viewCache } from './cache';
import { requireInput } from './input';
import { mergeCommandParts, commandPartsStatus } from './commandParts';
import { parseSession, type Cursor, type ViewData, type ViewChange, type ViewName, type DeviceSession, type CommandState } from './types';
function ulid() { const alphabet = '0123456789ABCDEFGHJKMNPQRSTVWXYZ'; let time = Date.now(), id = ''; for (let i = 0; i < 10; i++) {
    id = alphabet[time % 32] + id;
    time = Math.floor(time / 32);
} const random = crypto.getRandomValues(new Uint8Array(16)); for (const byte of random)
    id += alphabet[byte & 31]; return id; }
const finalCommandStates = new Set(['rejected', 'failed', 'observed', 'delivered', 'delivery_failed', 'indeterminate', 'expired']);
function rowID(value: unknown): string | undefined { if (!value || typeof value !== 'object')
    return; const row = value as {
    id?: string;
    chat?: {
        id: string;
    };
}; return row.id ?? row.chat?.id; }
export function reduceView(previous: ViewData, changes: ViewChange[]): ViewData {
    let data = { ...previous };
    for (const change of changes) {
        if (change.op === 'replace' && change.data) {
            data = change.data;
            continue;
        }
        const field = Array.isArray(data.chats) ? 'chats' : Array.isArray(data.items) ? 'items' : 'records';
        const rows = [...(data[field] ?? [])];
        const row = change.row ?? change.record, id = change.id ?? rowID(row), index = rows.findIndex(value => rowID(value) === id);
        if (change.op === 'remove' || change.op === 'delete') {
            if (index >= 0)
                rows.splice(index, 1);
        }
        else if (row) {
            if (index >= 0)
                rows[index] = row as never;
            else
                rows.push(row as never);
        }
        if (field === 'chats')
            rows.sort((a, b) => { const x = a as unknown as {
                last_activity_at?: string;
                id?: string;
                chat?: {
                    id: string;
                };
            }, y = b as typeof x; return String(y.last_activity_at ?? '').localeCompare(String(x.last_activity_at ?? '')) || String(x.chat?.id ?? x.id).localeCompare(String(y.chat?.id ?? y.id)); });
        if (field === 'items')
            rows.sort((a, b) => { const x = a as unknown as {
                id: string;
                data?: {
                    sent_at?: string;
                };
            }, y = b as typeof x; return String(x.data?.sent_at ?? '').localeCompare(String(y.data?.sent_at ?? '')) || x.id.localeCompare(y.id); });
        data = { ...data, [field]: rows, ...(change.newer !== undefined ? { newer: change.newer } : {}), ...(change.older !== undefined ? { older: change.older } : {}) };
    }
    return data;
}
type Subscription = {
    id: string;
    view: ViewName;
    args: Record<string, unknown>;
    cursor?: Cursor;
    data: () => ViewData;
    update: (data: ViewData) => void;
    cacheKey: string;
    generation: number;
    cache?: boolean;
    resetOnReconnect?: boolean;
};
export class LiveClient {
    private socket?: WebSocket;
    private subscriptions = new Map<string, Subscription>();
    private commands = new Map<string, CommandState>();
    private timer: ReturnType<typeof setTimeout> | undefined;
    private heartbeat: ReturnType<typeof setInterval> | undefined;
    private stopped = false;
    private attempt = 0;
    private lastFrame = 0;
    private connection = createSignal<'connecting' | 'connected' | 'offline'>('connecting');
    private commandRevision = createSignal(0);
    readonly status = this.connection[0];
    readonly pending = () => { this.commandRevision[0](); return [...this.commands.values()]; };
    constructor(private revoked: () => void, private event?: (frame: Record<string, unknown>) => void) { this.connect(); }
    private send(frame: unknown) { if (this.socket?.readyState !== WebSocket.OPEN)
        throw new Error('Unavailable'); this.socket.send(JSON.stringify(frame)); }
    private connect() {
        if (this.stopped)
            return;
        this.connection[1]('connecting');
        const socket = new WebSocket(new URL('/web/v1/live', location.href).href.replace(/^http/, 'ws'), 'web.1');
        this.socket = socket;
        socket.onopen = () => { this.lastFrame = Date.now(); this.send({ t: 'hello', v: 1 }); this.heartbeat = setInterval(() => { if (Date.now() - this.lastFrame > 45000) {
            socket.close();
            return;
        } if (socket.readyState === WebSocket.OPEN)
            this.send({ t: 'ping' }); }, 15000); };
        socket.onmessage = event => {
            if(this.stopped)return;
            this.lastFrame = Date.now();
            try {
                this.receive(JSON.parse(event.data));
            }
            catch {
                socket.close(4002, 'invalid frame');
            }
        };
        socket.onclose = event => {
            clearInterval(this.heartbeat);
            this.connection[1]('offline');
            if (event.reason === 'session_revoked') {
                this.revoke();
                return;
            }
            for (const command of this.commands.values())
                if (!command.disposition && !finalCommandStates.has(command.status))
                    this.commands.set(command.id, { ...command, status: 'indeterminate', transport_lost: true, error: { code: 'transport_lost' } });
            this.commandRevision[1](value => value + 1);
            if (!this.stopped) {
                void fetch('/web/v1/session', { cache: 'no-store', credentials: 'same-origin' }).then(response => { if (response.status === 401)
                    this.revoke(); }).catch(() => { });
                const delay = Math.min(30000, 250 * 2 ** Math.min(this.attempt++, 7)) * (.75 + Math.random() * .5);
                this.timer = setTimeout(() => this.connect(), delay);
            }
        };
        socket.onerror = () => socket.close();
    }
    private revoke() { this.close(); for (const sub of this.subscriptions.values()) {
        sub.generation++;
        sub.update({});
    } this.commands.clear(); this.commandRevision[1](value => value + 1); this.revoked(); }
    private receive(value: unknown) {
        if (!value || typeof value !== 'object' || typeof (value as {
            t?: unknown;
        }).t !== 'string')
            throw new Error('Invalid frame');
        const frame = value as Record<string, unknown>;
        if (frame.t === 'welcome') {
            const session: DeviceSession = parseSession(frame.session);
            if (session.cache === 'memory')
                viewCache.disable();
            this.attempt = 0;
            for (const sub of this.subscriptions.values())
                if (sub.resetOnReconnect) { sub.cursor = undefined; sub.generation++; sub.update({}); }
            this.connection[1]('connected');
            for (const sub of this.subscriptions.values())
                this.send({ t: 'sub', id: sub.id, view: sub.view, args: sub.args, cursor: sub.cursor });
            for (const command of this.commands.values()) {
                if (command.transport_lost || !command.disposition && !finalCommandStates.has(command.status))
                    this.reconcile(command.id);
            }
        }
        else if (frame.t === 'snap' || frame.t === 'delta') {
            if (typeof frame.sub !== 'string' || !frame.cursor || typeof frame.cursor !== 'object')
                throw new Error('Invalid view frame');
            const sub = this.subscriptions.get(frame.sub);
            if (!sub)
                return;
            const cursor = frame.cursor as Cursor;
            if (typeof cursor.epoch !== 'string' || typeof cursor.sequence !== 'number' || typeof cursor.subscription_revision !== 'number')
                throw new Error('Invalid cursor');
            if (frame.t === 'delta' && sub.cursor && cursor.epoch === sub.cursor.epoch && cursor.subscription_revision === sub.cursor.subscription_revision && cursor.sequence <= sub.cursor.sequence)
                return;
            if (frame.t === 'delta' && (!sub.cursor || cursor.epoch !== sub.cursor.epoch || cursor.subscription_revision !== sub.cursor.subscription_revision)) {
                sub.cursor = undefined;
                if (sub.resetOnReconnect) sub.update({});
                this.send({ t: 'unsub', id: sub.id });
                this.send({ t: 'sub', id: sub.id, view: sub.view, args: sub.args });
                return;
            }
            if (frame.t === 'snap' && (!frame.data || typeof frame.data !== 'object') || frame.t === 'delta' && !Array.isArray(frame.changes))
                throw new Error('Invalid data');
            sub.generation++;
            sub.cursor = cursor;
            sub.update(frame.t === 'snap' ? frame.data as ViewData : { ...reduceView(sub.data(), frame.changes as ViewChange[]), ...(frame.page && typeof frame.page === 'object' ? { next: undefined, before: undefined, after: undefined, unread_anchor: undefined, ...frame.page } : {}) });
            if(sub.cache!==false)void viewCache.write(sub.cacheKey, sub.data(), cursor).catch(() => { });
        }
        else if (frame.t === 'ack' || frame.t === 'cmd_status') {
            if (typeof frame.id !== 'string' || typeof frame.status !== 'string')
                throw new Error('Invalid command status');
            this.updateCommand(frame.id, frame.command, frame.status, frame.error, frame.t === 'cmd_status');
        }
        else if (frame.t === 'error' && typeof frame.id === 'string' && this.subscriptions.has(frame.id)) {
            const sub = this.subscriptions.get(frame.id)!;
            sub.generation++; sub.update({ error: typeof frame.code === 'string' ? frame.code : 'unavailable' });
            if (sub.cache !== false) void viewCache.write(sub.cacheKey, sub.data(), sub.cursor).catch(() => {});
        }
        else if (frame.t === 'error' && typeof frame.id === 'string')
            this.setCheckError(frame.id, typeof frame.code === 'string' ? frame.code : 'unavailable');
        else if (frame.t === 'session_revoked')
            this.revoke();
        else if (frame.t === 'error' && frame.code === 'reset') {
            for (const sub of this.subscriptions.values()) {
                sub.cursor = undefined;
                if (sub.resetOnReconnect) { sub.generation++; sub.update({}); }
                this.send({ t: 'unsub', id: sub.id });
                this.send({ t: 'sub', id: sub.id, view: sub.view, args: sub.args });
            }
        }
        this.event?.(frame);
    }
    private updateCommand(id: string, value: unknown, status: string, error: unknown, clearCheck = false) {
        const previous = this.commands.get(id);
        if (!previous) return;
        if (finalCommandStates.has(previous.status) && !previous.transport_lost && !finalCommandStates.has(status)) return;
        const record = value as Partial<CommandState> | undefined;
        const cursor = record?.cursor;
        if (!cursor && previous.cursor || cursor && previous.cursor && cursor.epoch === previous.cursor.epoch && cursor.subscription_revision === previous.cursor.subscription_revision && cursor.sequence < previous.cursor.sequence) return;
        const parts = previous.op === 'messages.send' ? mergeCommandParts(previous.parts, record?.parts, previous.args) : undefined;
        if (parts?.length && finalCommandStates.has(status)) status = commandPartsStatus(parts, status);
        this.commands.set(id, { ...previous, status, error, transport_lost: false, check_error: clearCheck ? undefined : previous.check_error,
            disposition: record?.disposition ?? previous.disposition,
            evidence: record?.evidence ?? previous.evidence,
            commit_seq: record?.commit_seq ?? previous.commit_seq,
            observed_record_ids: record?.observed_record_ids?.slice() ?? previous.observed_record_ids,
            parts,
            cursor: cursor ?? previous.cursor });
        this.commandRevision[1](value => value + 1);
    }
    private reconcile(id: string, force = false) {
        const subID = 'command:' + id;
        const existing = this.subscriptions.get(subID);
        if (existing) {
            if (!force) return;
            existing.cursor = undefined; existing.generation++; existing.update({});
            this.send({ t: 'unsub', id: subID });
            this.send({ t: 'sub', id: subID, view: existing.view, args: existing.args });
            return;
        }
        const [data, update] = createSignal<ViewData>({});
        const sub: Subscription = { id: subID, view: 'record', args: { collection: 'system.command', id }, data, update, cacheKey: subID, generation: 0, cache: false, resetOnReconnect: true };
        this.subscriptions.set(subID, sub);
        const original = update;
        sub.update = value => { original(value); const record = value.records?.[0]?.data; if (record && typeof record.state === 'string') {
            this.updateCommand(id, record, record.state, record.error);
        } };
        this.send({ t: 'sub', id: subID, view: sub.view, args: sub.args });
    }
    subscribe(view: ViewName, args: Record<string, unknown>, options: {cache?:boolean;cursor?:Cursor;resetOnReconnect?:boolean} = {}) {
        const id = ulid(), cacheKey = JSON.stringify([view, args]);
        const [data, update] = createSignal<ViewData>({});
        const sub: Subscription = { id, view, args, data, update, cacheKey, generation: 0, cache:options.cache,cursor:options.cursor,resetOnReconnect:options.resetOnReconnect };
        this.subscriptions.set(id, sub);
        if(options.cache!==false)void viewCache.read(cacheKey).then(cached => { if (cached && sub.generation === 0 && this.subscriptions.has(id)) {
            sub.cursor = cached.cursor;
            update(cached.data);
        } }).catch(() => { });
        if (untrack(this.status) === 'connected')
            this.send({ t: 'sub', id, view, args, ...(sub.cursor?{cursor:sub.cursor}:{}) });
        return { data, cursor: () => { data(); return sub.cursor; }, dispose: () => { this.subscriptions.delete(id); if (untrack(this.status) === 'connected')
                this.send({ t: 'unsub', id }); } };
    }
    command(op: string, args: unknown, optimistic?: unknown, expectedVersion?: number) {
        requireInput();
        if (this.status() !== 'connected' || this.socket?.readyState !== WebSocket.OPEN) throw new Error('Unavailable');
        const id = ulid();
        this.commands.set(id, { id, op, args, optimistic, expected_version: expectedVersion, status: 'pending' });
        try { this.send({ t: 'cmd', id, op, args, expected_version: expectedVersion }); }
        catch { this.commands.set(id, { ...this.commands.get(id)!, status: 'indeterminate', transport_lost: true, error: { code: 'transport_lost' } }); }
        this.commandRevision[1](value => value + 1);
        return id;
    }

    private setCheckError(id: string, code?: string) {
        const command = this.commands.get(id);
        if (!command || command.check_error === code) return;
        this.commands.set(id, { ...command, check_error: code });
        this.commandRevision[1](value => value + 1);
    }
    checkCommand(id: string) {
        if (!this.commands.has(id) || this.status() !== 'connected') throw new Error('Unavailable');
        this.setCheckError(id);
        try {
            this.send({ t: 'resolve', id });
            this.reconcile(id, true);
        } catch { this.setCheckError(id, 'unavailable'); }
    }
    reconnect() { this.socket?.close(4000,'reconnect'); }
    close() { this.stopped = true; clearTimeout(this.timer); clearInterval(this.heartbeat); this.socket?.close(); this.connection[1]('offline'); }
}
