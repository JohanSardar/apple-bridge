export type Handle = { id: string; name_resolved?: boolean; display_name: string; given_name?: string; family_name?: string; initials: string; address?: string; contact_id?: string; avatar?: { digest: string; variant: string } };
export type Handles = ReadonlyMap<string, Handle>;
export type Part = { index: number; kind: string; text?: string; attachment_id?: string; handle_id?: string; mention?: string; description?: string };
export function fields(value: unknown): Record<string, unknown> { return value && typeof value === 'object' ? value as Record<string, unknown> : {}; }
export function array(value: unknown): unknown[] { return Array.isArray(value) ? value : []; }
export function text(value: unknown): string { return typeof value === 'string' ? value : ''; }
