import { createSignal, onMount, onCleanup, Show } from 'solid-js';

type Pending = { id: string; code: string; client_name: string; redirect_host: string; scopes: string[]; expires_at: string };
export default function ConnectorPending() {
    const [pending, setPending] = createSignal<Pending>();
    const [error, setError] = createSignal('');
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    onCleanup(() => { stopped = true; clearTimeout(timer); });
    async function poll(value: Pending) {
        if (stopped) return;
        if (Date.now() >= Date.parse(value.expires_at)) { setError('This approval expired. Start again from your connector.'); return; }
        try {
            const response = await fetch('/oauth/pending/' + encodeURIComponent(value.id), { cache: 'no-store' });
            if (response.status === 410) { setError('This approval expired or was already used. Start again from your connector.'); return; }
            if (response.ok) {
                const result = await response.json() as { redirect: string };
                if (result.redirect && !stopped) { location.assign(result.redirect); return; }
            }
        } catch { }
        if (!stopped) timer = setTimeout(() => void poll(value), 1500);
    }
    onMount(() => {
        void (async () => {
            try {
                const response = await fetch('/oauth/pending' + location.search, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' });
                if (!response.ok) throw new Error();
                const value = await response.json() as Pending;
                if (!stopped) { setPending(value); void poll(value); }
            } catch { if (!stopped) setError('Cannot request approval. Wait a minute, then start again from your connector.'); }
        })();
    });
    return <main class="consent-page"><div class="consent-card"><h1>Approve on your Mac or iPhone</h1><Show when={pending()}>{value => <><h2>Code {value().code}</h2><p><strong>{value().client_name}</strong> wants to connect to Apple Bridge.</p><p class="secondary">Returns to {value().redirect_host}</p><p>Requested data: {value().scopes.join(', ')}</p><p>Open Apple Bridge on your Mac or iPhone. Match this code, then choose Allow or Deny. This request expires in five minutes.</p><p role="status">Waiting for approval…</p></>}</Show><Show when={error()}><p class="error" role="alert">{error()}</p></Show><a class="button subtle" href={'/signin?return_to=' + encodeURIComponent(location.pathname + location.search)}>Use a passkey instead</a></div></main>;
}
