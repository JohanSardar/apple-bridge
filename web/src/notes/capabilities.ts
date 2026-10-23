import { createSignal, onCleanup } from 'solid-js';
import { api } from '../auth/api';
import type { LiveClient } from '../data/live';
import type { BridgeRecord } from '../data/types';
import { object, text } from './store';
const operations = new Set(['notes.create', 'notes.update', 'notes.delete', 'notes.create_folder']);
export function noteCapabilities(live: LiveClient) {
    const [subscriptions, setSubscriptions] = createSignal<ReturnType<LiveClient['subscribe']>[]>([]);
    const [error, setError] = createSignal('Waiting for capability proof');
    let disposed = false;
    const discover = () => {
        void api<{ records: BridgeRecord[] }>('sources').then(result => {
            if (disposed) return;
            subscriptions().forEach(subscription => subscription.dispose());
            setSubscriptions(result.records.filter(record => record.collection === 'system.source' && record.source === 'notes').map(record => live.subscribe('record', { collection: 'system.source', id: record.id }, { cache: false, resetOnReconnect: true })));
            setError('Notes actions unavailable');
        }).catch(() => { if (!disposed) setError('Couldn’t check available actions'); });
    };
    discover();
    onCleanup(() => { disposed = true; subscriptions().forEach(subscription => subscription.dispose()); });
    const proof = (operation: string) => {
        if (live.status() !== 'connected') return { proven: false, reason: 'Waiting for connection' };
        if (!operations.has(operation)) return { proven: false, reason: 'Unsupported Notes action' };
        for (const subscription of subscriptions()) {
            for (const record of subscription.data().records ?? []) {
                const source = record.data, coverage = object(source.coverage);
                if (source.source !== 'notes' || source.state !== 'ready') continue;
                const entries = Array.isArray(coverage.operations) ? coverage.operations : [];
                const explicit = entries.map(object).find(entry => entry.operation === operation);
                if (operation === 'notes.delete' && explicit?.proven !== true) return { proven: false, reason: text(explicit?.reason) || 'capability_unproven' };
                if (explicit?.proven === false) return { proven: false, reason: text(explicit.reason) || 'Action unavailable on this Mac' };
                if (Array.isArray(source.capabilities) && source.capabilities.includes(operation)) return { proven: true, reason: text(explicit?.reason) || 'advertised_notes_operation' };
            }
        }
        return { proven: false, reason: error() };
    };
    const state = () => subscriptions().flatMap(subscription => subscription.data().records ?? []).find(record => record.data.source === 'notes')?.data.state;
    return { state, proof, allowed: (operation: string) => proof(operation).proven };
}
