import { render } from 'solid-js/web';
import { lazy, onMount, Show, Suspense } from 'solid-js';
import './design/tokens.css';
import './design/system.css';
import './app/app.css';
import './app/preferences';
import { router } from './app/router';
import { auth, loadSession } from './auth/session';
import Shell from './app/Shell';
import { strings as s } from './app/strings';
const AuthPage = lazy(() => import('./auth/AuthPage'));
const Settings = lazy(() => import('./settings/Settings'));
const Fixtures = lazy(() => import('./app/Fixtures'));
function App() {
    onMount(() => { void loadSession(); });
    return <><a class="skip-link" href="#main-content">{s.skipContent}</a><div id="main-content" tabindex="-1"><Suspense fallback={<main class="loading-page" aria-busy="true" aria-label={s.loading}/>}><Show when={!auth.error()} fallback={<main class="empty"><h1>{s.connectionFailed}</h1><button class="button" onClick={() => void auth.retry()}>{s.tryAgain}</button></main>}><Show when={auth.session() !== undefined} fallback={<main class="loading-page" aria-busy="true" aria-label={s.loading}/>}><Show when={auth.session() && router.route().path !== '/enroll'} fallback={<AuthPage />}><Show when={!router.route().path.startsWith('/__fixtures/')} fallback={<Fixtures />}><Show when={!router.route().path.startsWith('/settings')} fallback={<Settings />}><Shell /></Show></Show></Show></Show></Show></Suspense></div></>;
}
render(() => <App />, document.getElementById('root')!);
