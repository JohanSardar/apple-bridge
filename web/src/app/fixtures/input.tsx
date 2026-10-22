import { auth } from '../../auth/session';
import { withInput } from '../../data/input';
import { FixtureFrame, useFixtureStatus } from '../FixtureFrame';
function Input() {
    const setStatus = useFixtureStatus(), live = auth.live();
    const command = (event: Event) => {
        try { withInput(event, () => live?.command('fixture.command', {})); setStatus('Submitted'); }
        catch (error) { setStatus((error as Error).message); }
    };
    return <><button class="button" onClick={command}>Submit fixture command</button><button class="button" onClick={event => { setTimeout(() => command(event), 0); }}>Delayed fixture command</button><button class="button" onClick={() => command({ isTrusted: true, eventPhase: Event.BUBBLING_PHASE, type: 'click' } as Event)}>Forged fixture context</button><label><input type="checkbox" aria-label="Programmatic change" onChange={command}/>Programmatic change</label></>;
}
export default function InputFixture() { return <FixtureFrame><Input/></FixtureFrame>; }
