import { For, Show } from 'solid-js';
import type { createParityCommands } from './commands';
export function ParityNotices(props: { commands: ReturnType<typeof createParityCommands> }) {
    return <div class="parity-notices" aria-live="polite"><Show when={props.commands.problem()}><p role="status">{props.commands.problem()}</p></Show><For each={props.commands.notices()}>{notice => <p role="status">{notice.text}<Show when={notice.check}><button type="button" onClick={() => props.commands.check(notice.id)}>Check Change</button></Show></p>}</For></div>;
}
