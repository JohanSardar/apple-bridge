import { Show } from 'solid-js';
import { auth } from '../../auth/session';
import { createCapabilities } from './capabilities';
export function SourcesFixture() {
    const live = auth.live()!;
    const capabilities = createCapabilities(live);
    return <section aria-label="Source proofs"><output aria-label="Source connection">{live.status()}</output><output aria-label="Reaction proof">{capabilities.proof('messages.react').reason}</output><Show when={capabilities.allowed('messages.react')}><button class="button">React</button></Show><button class="button" onClick={() => live.reconnect()}>Reconnect source proofs</button></section>;
}
