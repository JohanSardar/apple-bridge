import type { LiveClient } from '../../data/live';
import type { ChatSummary } from '../../data/types';
import { capturedMute, createParityCommands } from './commands';
export function ParitySwipeAction(props: { live: LiveClient; chat: ChatSummary; close: () => void }) {
    const commands = createParityCommands(props.live, () => [props.chat.chat.id]);
    const record = () => commands.record(props.chat.chat.id);
    const muted = () => record() ? capturedMute(record()!) : capturedMute(props.chat.chat);
    return <button class="parity-swipe-mute" disabled={!commands.can('mute', record())} onClick={event => {
        const target = record();
        if (target && commands.submit(event, 'mute', target, { muted: !muted() })) props.close();
    }}>{muted() ? 'Show Alerts' : 'Hide Alerts'}</button>;
}
