import { createSignal, onCleanup } from 'solid-js';
import { api } from '../../auth/api';
import type { LiveClient } from '../../data/live';
import type { BridgeRecord } from '../../data/types';
type OperationProof = { operation: string; proven: boolean; reason: string };
type SourceData = { source: string; state: string; capabilities?: string[]; coverage?: { operations?: OperationProof[] } };
export type CapabilityProof = { proven: boolean; reason: string };
export function createCapabilities(live: LiveClient, source = 'messages') {
    const [subscriptions, setSubscriptions] = createSignal<ReturnType<LiveClient['subscribe']>[]>([]);
    const [error, setError] = createSignal(false);
    let disposed = false;
    void api<{ records: BridgeRecord[] }>('sources').then(result => {
        if (disposed) return;
        setSubscriptions([...new Set(result.records.filter(record => record.collection === 'system.source' && record.source === source).map(record => record.id))].map(id => live.subscribe('record', { collection: 'system.source', id }, { cache: false, resetOnReconnect: true })));
    }).catch(() => { if (!disposed) setError(true); });
    const proof = (operation: string): CapabilityProof => {
        if (live.status() !== 'connected') return { proven: false, reason: 'Waiting for connection' };
        for (const subscription of subscriptions()) {
            for (const record of subscription.data().records ?? []) {
                const source = record.data as unknown as SourceData;
                if (source.source !== operation.split('.')[0]) continue;
                const operationProof = source.coverage?.operations?.find(value => value.operation === operation);
                if (source.state !== 'ready') return { proven: false, reason: 'Source unavailable' };
                if (operationProof?.reason) return { proven: operationProof.proven === true && source.capabilities?.includes(operation) === true, reason: operationProof.reason };
            }
        }
        return { proven: false, reason: error() ? 'Couldn’t check available actions' : 'Waiting for capability proof' };
    };
    const dispose = () => { disposed = true; for (const subscription of subscriptions()) subscription.dispose(); setSubscriptions([]); };
    onCleanup(dispose);
    return { proof, allowed: (operation: string) => proof(operation).proven, dispose };
}
