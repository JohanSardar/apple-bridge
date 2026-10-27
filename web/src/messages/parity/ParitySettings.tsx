import { Show } from 'solid-js';
import type { LiveClient } from '../../data/live';
import type { BridgeRecord } from '../../data/types';
import { capturedMute, createParityCommands } from './commands';
import { ParityNotices } from './ParityNotices';
import { chatService } from '../store/message';
import './parity.css';
export function ConversationSettings(props: { live: LiveClient; chatID: string }) {
    const commands = createParityCommands(props.live, () => [props.chatID]);
    return <><Show when={commands.record(props.chatID)}>{record => <ParitySettings record={record()} commands={commands}/>}</Show><ParityNotices commands={commands}/></>;
}
export function ParitySettings(props: { record: BridgeRecord; commands: ReturnType<typeof createParityCommands> }) {
    const deleted = () => Boolean(props.record.data.deleted_at);
    const receipts = () => String(props.record.data.read_receipts);
    const pin = () => typeof props.record.data.pinned_index === 'number';
    const visible = () => !deleted() && (props.commands.allowed('pin') || props.commands.allowed('mute') && capturedMute(props.record) !== undefined || props.commands.allowed('set_read_receipts') && chatService(props.record) === 'iMessage' && ['default', 'on', 'off'].includes(receipts()));
    return <Show when={visible()}><section class="parity-settings" aria-label="Conversation settings">
        <Show when={props.commands.allowed('pin')}><button class="parity-setting-row" disabled={!props.commands.can('pin', props.record)} onClick={event => props.commands.submit(event, 'pin', props.record, { pinned_index: pin() ? -1 : 0 })}><span>{pin() ? 'Unpin Conversation' : 'Pin Conversation'}</span></button></Show>
        <Show when={props.commands.allowed('mute') && capturedMute(props.record) !== undefined}><div class="parity-setting-row"><span id={'mute-label-' + props.record.id}>Hide Alerts</span><button type="button" class="parity-toggle" role="switch" aria-labelledby={'mute-label-' + props.record.id} aria-checked={capturedMute(props.record)} disabled={!props.commands.can('mute', props.record)} onClick={event => props.commands.submit(event, 'mute', props.record, { muted: !capturedMute(props.record) })}><span class="parity-track" aria-hidden="true"><i/></span></button></div></Show>
        <Show when={props.commands.allowed('set_read_receipts') && chatService(props.record) === 'iMessage' && ['default', 'on', 'off'].includes(receipts())}><div class="parity-setting-row"><span id={'receipt-label-' + props.record.id}>Send Read Receipts<Show when={receipts() === 'default'}><small>Using the default setting</small></Show></span><button type="button" class="parity-toggle" role="checkbox" aria-labelledby={'receipt-label-' + props.record.id} aria-checked={receipts() === 'default' ? 'mixed' : receipts() === 'on'} disabled={!props.commands.can('set_read_receipts', props.record)} onClick={event => props.commands.submit(event, 'set_read_receipts', props.record, { read_receipts: receipts() === 'on' ? 'off' : 'on' })}><span class="parity-track" aria-hidden="true"><i/></span></button></div><button type="button" class="parity-default" style={{visibility:receipts() === 'default' ? 'hidden' : 'visible'}} tabindex={receipts() === 'default' ? -1 : undefined} aria-hidden={receipts() === 'default'} disabled={!props.commands.can('set_read_receipts', props.record)} onClick={event => props.commands.submit(event, 'set_read_receipts', props.record, { read_receipts: 'default' })}>Use Default Read Receipt Setting</button></Show>
    </section></Show>;
}
