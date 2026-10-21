import { createEffect, For, Show } from 'solid-js';
import { strings as s } from '../app/strings';
import { auth } from '../auth/session';
import { Icon } from '../design/Icon';
import { createHandlesCache } from '../messages/store/handles';
import { array, fields, text } from '../messages/store/types';
export function ApprovalPreview(props: { arguments: Record<string, unknown>; recipient?: string }) {
    const live = auth.live(), handles = live ? createHandlesCache(live) : undefined;
    const ids = () => array(props.arguments.handles).map(text).filter(Boolean);
    createEffect(() => handles?.ensure(ids()));
    const recipients = () => props.recipient || ids().map(id => handles?.read().get(id)?.display_name || s.unknownRecipient).join(', ') || s.unknownRecipient;
    return <div class="approval-preview"><p class="approval-recipient"><span class="secondary">{s.to}</span> <strong>{recipients()}</strong></p><Show when={text(props.arguments.text)}>{value => <p class="approval-bubble">{value()}</p>}</Show><Show when={array(props.arguments.files).length}><div class="attachment-chips"><For each={array(props.arguments.files)}>{file => <span class="attachment-chip"><Icon name="notes" />{text(fields(file).name) || s.attachment}</span>}</For></div></Show></div>;
}
