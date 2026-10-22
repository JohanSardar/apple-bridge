import type { CommandPart } from './types';
const dispositions = new Set(['observed', 'delivered', 'delivery_failed', 'failed_no_effect', 'indeterminate']);
export function mergeCommandParts(previous: readonly CommandPart[] | undefined, value: unknown, args: unknown): readonly CommandPart[] | undefined {
    if (!Array.isArray(value) || value.length < 1 || value.length > 11 || !args || typeof args !== 'object') return previous;
    const input = args as {text?:string;files?:{upload_id:string}[]};
    const expected = [...(input.text ? [{kind:'text',upload_id:undefined}] : []), ...(input.files ?? []).map(file => ({kind:'attachment',upload_id:file.upload_id}))];
    if (value.length !== expected.length) return previous;
    const ids = new Set<string>();
    const next: CommandPart[] = [];
    for (let index = 0; index < value.length; index++) {
        const part = value[index] as CommandPart | undefined, spec = expected[index];
        if (!part || part.index !== index || part.kind !== spec.kind || part.upload_id !== spec.upload_id || !dispositions.has(part.disposition)) return previous;
        if (part.record_id !== undefined && (typeof part.record_id !== 'string' || !part.record_id || ['indeterminate','failed_no_effect'].includes(part.disposition))) return previous;
        if (part.record_id && ids.has(part.record_id)) return previous;
        if (part.record_id) ids.add(part.record_id);
        const prior = previous?.[index];
        const strengthens = !prior || prior.disposition === part.disposition || prior.disposition === 'indeterminate' || prior.disposition === 'observed' && ['delivered','delivery_failed'].includes(part.disposition);
        next.push(prior && (!strengthens || prior.record_id && prior.record_id !== part.record_id) ? prior : {...part});
    }
    const bound = next.flatMap(part => part.record_id ? [part.record_id] : []);
    return new Set(bound).size === bound.length ? next : previous;
}
export function commandPartsStatus(parts: readonly CommandPart[], fallback: string): string {
    if (parts.some(part => ['delivery_failed','failed_no_effect'].includes(part.disposition))) return 'delivery_failed';
    if (parts.every(part => ['observed','delivered'].includes(part.disposition))) return parts.every(part => part.disposition === 'delivered') ? 'delivered' : 'observed';
    return fallback === 'observed' || fallback === 'delivered' ? fallback : 'indeterminate';
}
