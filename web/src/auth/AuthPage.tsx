import { createSignal, Show } from 'solid-js';
import { passkey } from './api';
import { establishSession } from './session';
import { router } from '../app/router';
import { strings as s } from '../app/strings';
import { Icon } from '../design/Icon';
const enrollmentToken = location.pathname === '/enroll' ? location.hash.slice(1) : '';
if (enrollmentToken)
    history.replaceState(history.state, '', location.pathname);
export default function AuthPage() {
    const enroll = () => router.route().path === '/enroll';
    const [kind, setKind] = createSignal<'trusted' | 'public'>('public'), [pending, setPending] = createSignal(false), [error, setError] = createSignal('');
    async function submit() { setPending(true); setError(''); try {
        const session = await passkey(enroll() ? 'enroll' : 'login', enroll() ? 'trusted' : kind(), enrollmentToken);
        await establishSession(session);
        const returnTo = new URLSearchParams(location.search).get('return_to');
        if (returnTo?.startsWith('/oauth/authorize?')) location.assign(returnTo);
        else router.navigate('/messages', true);
    }
    catch (error) {
        setError(error instanceof DOMException && error.name === 'NotAllowedError' ? s.passkeyCancelled : s.unavailable);
    }
    finally {
        setPending(false);
    } }
    return <main class="auth-page"><div class="auth-card"><div class="auth-mark"><Icon name="key"/></div><h1>{enroll() ? s.enroll : s.signIn}</h1><p class="secondary">{enroll() ? s.enrollBody : s.signInBody}</p><Show when={!enroll()}><fieldset><legend>{s.trustedQuestion}</legend><label class="choice"><input type="radio" name="kind" value="trusted" checked={kind() === 'trusted'} onChange={() => setKind('trusted')}/><span><strong>{s.trusted}</strong><span class="secondary">{s.trustedHint}</span></span></label><label class="choice"><input type="radio" name="kind" value="public" checked={kind() === 'public'} onChange={() => setKind('public')}/><span><strong>{s.public}</strong><span class="secondary">{s.publicHint}</span></span></label></fieldset></Show><Show when={enroll() && !enrollmentToken}><p role="alert">{s.finishOnMac}</p></Show><Show when={error()}><p class="error" role="alert">{error()}</p></Show><button class="button" disabled={pending() || (enroll() && !enrollmentToken)} onClick={() => void submit()}>{pending() ? s.passkeyWaiting : enroll() ? s.continue : s.signIn}</button><p class="caption">{enroll() ? s.passkeyPrivate : s.passkeyDevice}</p></div></main>;
}
