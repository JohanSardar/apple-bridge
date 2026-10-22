export type Cursor = {
    epoch: string;
    subscription_revision: number;
    sequence: number;
};
export type DeviceSession = {
    device_id: string;
    kind: 'trusted' | 'public';
    expires_at: string;
    cache: 'persistent' | 'memory';
};
export type BridgeRecord<T = Record<string, unknown>> = {
    id: string;
    collection: string;
    source: string;
    version: number;
    data: T;
};
export type ChatSummary = {
    known_sender?: boolean;
    pin_preview?: {message_id:string;text_preview:string;at:string}|null;
    chat: BridgeRecord;
    title: string;
    avatar_handles: {
        id: string;
        contact_id: string | null;
    }[];
    last_message: {
        id: string;
        sent_at: string;
        is_from_me: boolean;
        sender_handle_id?: string;
        preview: {
            kind: 'text' | 'attachments' | 'reaction' | 'group_event' | 'unsent' | 'edited';
            text?: string;
            group_event?: unknown;
            attachment_kinds?: string[];
            count?: number;
            reaction?: { verb: string; target_excerpt: string; kind?: string; removed?: boolean };
        };
    } | null;
    last_activity_at: string;
    unread_count: number;
    has_unseen_mention: boolean;
    muted: null | boolean;
    pinned_index: null | number;
};
export type ThreadItem = BridgeRecord<{
    sent_at: string;
    [key: string]: unknown;
}> & {
    reactions: BridgeRecord[];
    attachments: BridgeRecord[];
    reply_count: number;
};
export type ViewName = 'chats' | 'thread' | 'record' | 'collection' | 'reminders.lists' | 'reminders.items';
export type ViewData = {
    chats?: ChatSummary[];
    items?: ThreadItem[];
    records?: BridgeRecord[];
    before?: string;
    after?: string;
    next?: string;
    unread_anchor?: string;
    newer?: number;
    older?: number;
    [key: string]: unknown;
};
export type ViewChange = {
    op: 'upsert' | 'remove' | 'delete' | 'replace';
    id?: string;
    row?: unknown;
    record?: BridgeRecord;
    data?: ViewData;
    newer?: number;
    older?: number;
};
export type CommandPart = {
    index: number;
    kind: 'text' | 'attachment';
    upload_id?: string;
    record_id?: string;
    disposition: 'observed' | 'delivered' | 'delivery_failed' | 'failed_no_effect' | 'indeterminate';
};
export type CommandState = {
    id: string;
    op: string;
    args: unknown;
    expected_version?: number;
    status: string;
    commit_seq?: number;
    observed_record_ids?: string[];
    parts?: readonly CommandPart[];
    cursor?: Cursor;
    transport_lost?: boolean;
    check_error?: string;
    optimistic?: unknown;
    error?: unknown;
    disposition?: string;
};
export function parseSession(value: unknown): DeviceSession {
    if (!value || typeof value !== 'object')
        throw new Error('Invalid session');
    const session = value as DeviceSession;
    if (typeof session.device_id !== 'string' || typeof session.expires_at !== 'string' || !['trusted', 'public'].includes(session.kind) || session.cache !== (session.kind === 'trusted' ? 'persistent' : 'memory'))
        throw new Error('Invalid session');
    return session;
}
