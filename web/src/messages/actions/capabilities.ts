import { createEffect, createSignal, onCleanup } from 'solid-js';
import { api } from '../../auth/api';
import type { LiveClient } from '../../data/live';
import type { BridgeRecord } from '../../data/types';
type OperationProof = { operation: string; proven: boolean; reason: string };
type SourceData = { source: string; state: string; capabilities?: string[]; coverage?: { operations?: OperationProof[] } };
export type CapabilityProof = { proven: boolean; reason: string };
export function createCapabilities(live: LiveClient, source = 'messages') {
    const [subscriptions, setSubscriptions] = createSignal<ReturnType<LiveClient['subscribe']>[]>([]);
    const [error, setError] = createSignal(false);
    const [failureRevision, setFailureRevision] = createSignal(0);
    let disposed = false, discovered = false, inFlight = false, failures = 0, retryAt = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const discover = () => {
        if (disposed || discovered || inFlight) return;
        inFlight = true;
        void api<{ records: BridgeRecord[] }>('sources').then(result => {
            if (disposed) return;
            setSubscriptions([...new Set(result.records.filter(record => record.collection === 'system.source' && record.source === source).map(record => record.id))].map(id => live.subscribe('record', { collection: 'system.source', id }, { cache: false, resetOnReconnect: true })));
            discovered = true;
            failures = 0;
            retryAt = 0;
            setError(false);
        }).catch(() => {
            if (disposed) return;
            failures = Math.min(failures + 1, 8);
            retryAt = Date.now() + Math.min(30000, 1000 * 2 ** (failures - 1));
            setError(true);
            setFailureRevision(value => value + 1);
        }).finally(() => { inFlight = false; });
    };
    discover();
    createEffect(() => {
        failureRevision();
        const connected = live.status() === 'connected';
        clearTimeout(timer);
        if (connected && failures > 0 && !discovered)
            timer = setTimeout(() => { if (live.status() === 'connected') discover(); }, Math.max(0, retryAt - Date.now()));
    });
    const proof = (operation: string): CapabilityProof => {
        if (live.status() !== 'connected') return { proven: false, reason: 'Waiting for connection' };
        for (const subscription of subscriptions()) {
            for (const record of subscription.data().records ?? []) {
                const source = record.data as unknown as SourceData;
                if (source.source !== operation.split('.')[0]) continue;
                const operationProof = source.coverage?.operations?.find(value => value.operation === operation);
                if (source.state !== 'ready') return { proven: false, reason: 'Source unavailable' };
                if (operationProof) return { proven: operationProof.proven === true && source.capabilities?.includes(operation) === true, reason: operationProof.reason };
            }
        }
        return { proven: false, reason: error() ? 'Couldn’t check available actions' : 'Waiting for capability proof' };
    };
    const dispose = () => { disposed = true; clearTimeout(timer); for (const subscription of subscriptions()) subscription.dispose(); setSubscriptions([]); };
    onCleanup(dispose);
    return { proof, allowed: (operation: string) => proof(operation).proven, dispose };
}
