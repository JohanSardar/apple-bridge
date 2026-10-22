import { createSignal } from 'solid-js';
import { VirtualList, type VirtualListHandle } from '../../virtual/VirtualList';
import { FixtureFrame } from '../FixtureFrame';
type Row = { id: string; text: string };
export default function VirtualFixture() {
    const [rows, setRows] = createSignal<Row[]>(Array.from({ length: 100000 }, (_, i) => ({ id: String(i), text: `Row ${i} · ` + 'Variable height content. '.repeat(i % 7 + 1) })));
    const [bottom, setBottom] = createSignal(false);
    let handle: VirtualListHandle | undefined;
    return <FixtureFrame><header><button class="button subtle" onClick={() => { setRows(previous => [...Array.from({ length: 20 }, (_, i) => ({ id: 'older-' + i, text: 'Older row ' + i })), ...previous]); }}>Prepend 20</button><button class="button subtle" onClick={() => setRows(previous => [...previous, { id: 'appended-' + previous.length, text: 'Appended row' }])}>Append</button><button class="button subtle" onClick={() => handle?.scrollToIndex(50000)}>Middle</button><button class="button subtle" onClick={() => { setBottom(!bottom()); handle?.scrollToIndex(rows().length - 1, 'end'); }}>Bottom mode</button></header><VirtualList items={rows()} keyFor={row => row.id} estimate={row => 48 + (Number(row.id) || 0) % 7 * 16} bottomAnchored={bottom()} label="100000 variable-height rows" ref={value => { handle = value; }} render={row => <div class="fixture-row">{row.text}</div>}/></FixtureFrame>;
}
