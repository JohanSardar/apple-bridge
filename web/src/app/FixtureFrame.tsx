import { createContext, createEffect, createSignal, Show, useContext, type JSX } from 'solid-js';
import { auth } from '../auth/session';
import { router } from './router';
const Status = createContext<(value: string) => void>();
export function useFixtureStatus() {
    const update = useContext(Status);
    if (!update) throw new Error('Fixture frame is required');
    return update;
}
export function FixtureFrame(props: { children?: JSX.Element; live?: boolean }) {
    const [status, setStatus] = createSignal('Ready'), live = auth.live();
    createEffect(() => { const commands = live?.pending(); if (commands?.length) setStatus(commands.at(-1)!.status); });
    return <Status.Provider value={setStatus}><main class="fixture"><header><button class="button subtle" onClick={() => router.navigate('/messages')}>Back to shell</button><h1>Foundation fixture</h1></header>{props.children}<Show when={!props.live}><output role="status">{status()}</output></Show></main></Status.Provider>;
}
