import { render } from 'solid-js/web';
import { createEffect, lazy, onMount, Show, Suspense } from 'solid-js';
import './design/tokens.css';
import './design/system.css';
import './app/app.css';
import './app/preferences';
import { router } from './app/router';
import { auth, loadSession } from './auth/session';
import Shell from './app/Shell';
import { isMac } from './app/keyboard';
import { Icon } from './design/Icon';
import { strings as s } from './app/strings';
const Calendar = lazy(() => import('./calendar/Calendar'));
const Reminders = lazy(() => import('./reminders/Reminders'));
import { createChatsStore } from './messages/store/chats';
import { trackUnread } from './messages/store/badge';
const AuthPage = lazy(() => import('./auth/AuthPage'));
const Consent = lazy(() => import('./settings/Consent'));
const Settings = lazy(() => import('./settings/Settings'));
const Fixtures = lazy(() => import('./app/Fixtures'));
function App() {
    createEffect(()=>{const live=auth.live();if(live)trackUnread(createChatsStore(live).unread);});
    onMount(() => { void loadSession().then(() => { const target = new URLSearchParams(location.search).get('return_to'); if (auth.session() && location.pathname === '/signin' && target?.startsWith('/oauth/authorize?')) location.replace(target); }); });
    return <><a class="skip-link" href="#main-content">{s.skipContent}</a><div id="main-content" tabindex="-1"><Suspense fallback={<main class="loading-page" aria-busy="true" aria-label={s.loading}/>}><Show when={!auth.error()} fallback={<main class="empty"><h1>{s.connectionFailed}</h1><button class="button" onClick={() => void auth.retry()}>{s.tryAgain}</button></main>}><Show when={auth.session() !== undefined} fallback={<main class="loading-page" aria-busy="true" aria-label={s.loading}/>}><Show when={auth.session() && router.route().path !== '/enroll'} fallback={<AuthPage />}><Show when={router.route().path !== '/oauth/authorize'} fallback={<Consent />}><Show when={!router.route().path.startsWith('/__fixtures/')} fallback={<Fixtures />}><Show when={!router.route().path.startsWith('/settings')} fallback={<Settings />}><Show when={router.route().path.startsWith('/calendar')} fallback={<Show when={router.route().path.startsWith('/reminders')} fallback={<Shell />}><Reminders/></Show>}><Calendar/></Show></Show></Show></Show></Show></Show></Show></Suspense><Show when={auth.notice()}><div class="session-notice glass" role="alert"><span>{auth.notice()}</span><button class="icon-button" aria-label={s.dismiss} onClick={()=>auth.dismissNotice()}><Icon name="close"/></button></div></Show></div></>;
}
if(isMac)document.documentElement.dataset.platform='apple';
render(() => <App />, document.getElementById('root')!);
