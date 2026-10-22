import { viewCache } from '../../data/cache';
import { FixtureFrame, useFixtureStatus } from '../FixtureFrame';
function Cache() {
    const setStatus = useFixtureStatus();
    return <button class="button" onClick={() => { void viewCache.write('eviction-new', { records: [] }).then(() => setStatus('Persisted')); }}>Persist fixture view</button>;
}
export default function CacheFixture() { return <FixtureFrame><Cache/></FixtureFrame>; }
