import { createEffect, createMemo, createSignal, For, onCleanup, onMount, type JSX } from 'solid-js';
import { beginWork } from '../motion/work';
import { Positions } from './positions';
export type VirtualListHandle = {
    scrollToIndex: (index: number, align?: 'start' | 'end') => void;
    anchor: () => {
        key: string;
        offset: number;
    } | undefined;
};
export function VirtualList<T>(props: {
    items: readonly T[];
    keyFor: (item: T) => string;
    estimate?: (item: T) => number;
    render: (item: T, index: number) => JSX.Element;
    bottomAnchored?: boolean;
    fixedHeights?: boolean;
    selectedKey?: string;
    keepTop?: boolean;
    onActive?: (item:T)=>void;
    onBottom?: (value:boolean)=>void;
    label: string;
    ref?: (handle: VirtualListHandle) => void;
    onVisible?: (start: number, end: number) => void;
}) {
    const controlled='selectedKey' in props;
    const selected=()=>{revision();return controlled?(props.selectedKey?keyIndex.get(props.selectedKey)??-1:-1):active();};
    let viewport!: HTMLDivElement;
    let positions = new Positions([]), items: readonly T[] = [];
    let keyIndex = new Map<string, number>();
    const cache = new Map<string, number>();
    const [revision, setRevision] = createSignal(0), [scroll, setScroll] = createSignal(0), [height, setHeight] = createSignal(800), [active, setActive] = createSignal(0);
    let observer: ResizeObserver | undefined, frame = 0;
    const range = createMemo(() => { revision(); const top = scroll(); const start = Math.max(0, positions.indexAt(top) - 6), end = Math.min(items.length, positions.indexAt(top + height()) + 7); return { start, end }; });
    const anchor = () => { if (!items.length)
        return; const index = positions.indexAt(viewport.scrollTop); return { key: props.keyFor(items[index]), offset: viewport.scrollTop - positions.offset(index) }; };
    const refresh = () => { setScroll(viewport.scrollTop); setHeight(viewport.clientHeight); setRevision(value => value + 1); };
    const scrollToIndex = (index: number, align: 'start' | 'end' = 'start') => { viewport.scrollTop = align === 'end' ? positions.offset(index + 1) - viewport.clientHeight : positions.offset(index); refresh(); if(align==='end'&&index===items.length-1)queueMicrotask(()=>{viewport.scrollTop=viewport.scrollHeight;refresh();}); };
    createEffect(() => {
        const next = props.items;
        const finish=beginWork('virtual-items');
        const previous = viewport ? anchor() : undefined;
        const atTop=viewport ? viewport.scrollTop<=2 : true;
        const atBottom = viewport ? viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight <= 2 : false;
        const first = items.length === 0;
        items = next;
        keyIndex = new Map(items.map((item, index) => [props.keyFor(item), index]));
        positions = new Positions(items.map(item => { const estimate=(props.fixedHeights ? props.estimate?.(item) : cache.get(props.keyFor(item)) ?? props.estimate?.(item)) ?? 60; return Number.isFinite(estimate)&&estimate>0?estimate:60; }));
        if (viewport) {
            if (props.bottomAnchored && (atBottom || first)) {
                viewport.scrollTop = Math.max(0, positions.total - viewport.clientHeight);
                queueMicrotask(() => { viewport.scrollTop = viewport.scrollHeight; refresh(); });
            }
            else if(props.keepTop&&atTop)viewport.scrollTop=0;
            else if (previous) {
                const index = (keyIndex.get(previous.key) ?? -1);
                if (index >= 0)
                    viewport.scrollTop = positions.offset(index) + previous.offset;
            }
            refresh();
        }
        else
            setRevision(value => value + 1);
        finish();
    });
    onMount(() => {
        props.ref?.({ scrollToIndex, anchor });
        observer = new ResizeObserver(entries => {
            const saved = anchor(), atBottom = viewport.scrollHeight - viewport.scrollTop - height() <= 2;
            let changed = false, resized=false;
            for (const entry of entries) {
                if (entry.target === viewport) {
                    resized=true;
                    setHeight(viewport.clientHeight);
                    continue;
                }
                const row = entry.target as HTMLElement, index = Number(row.dataset.index), item = items[index];
                if (!item || row.dataset.key !== props.keyFor(item))
                    continue;
                const measured = entry.borderBoxSize?.[0]?.blockSize ?? entry.target.getBoundingClientRect().height;
                if (Math.abs(positions.height(index) - measured) > .1) {
                    cache.set(props.keyFor(item), measured);
                    positions.measure(index, measured);
                    changed = true;
                }
            }
            if (changed||resized) {
                if (props.bottomAnchored && atBottom) {
                    viewport.scrollTop = Math.max(0, positions.total - viewport.clientHeight);
                    queueMicrotask(()=>{viewport.scrollTop=viewport.scrollHeight;refresh();});
                }
                else if (saved) {
                    const index = (keyIndex.get(saved.key) ?? -1);
                    if (index >= 0)
                        viewport.scrollTop = positions.offset(index) + saved.offset;
                }
                refresh();
            }
        });
        observer.observe(viewport);
        if(!props.fixedHeights)viewport.querySelectorAll('[data-index]').forEach(element => observer?.observe(element));
        if (props.bottomAnchored)
            scrollToIndex(items.length - 1, 'end');
        refresh();
    });
    onCleanup(() => { observer?.disconnect(); cancelAnimationFrame(frame); });
    createEffect(() => { const value = range(); props.onVisible?.(value.start, value.end); props.onBottom?.(positions.total-scroll()-height()<=2); });
    const keys = createMemo(() => { revision(); return items.slice(range().start,range().end).map(props.keyFor); });
    const activeDescendant=createMemo(()=>{revision();const item=items[selected()];if(!item)return;const key=props.keyFor(item);return keys().includes(key)?'virtual-row-'+key:undefined;});
    return <div ref={viewport} class="virtual-viewport" data-scroll={props.label} tabindex="0" role="listbox" aria-label={props.label} aria-activedescendant={activeDescendant()} onScroll={() => { if (!frame)
        frame = requestAnimationFrame(() => { frame = 0; const finish=beginWork('virtual-scroll'); setScroll(viewport.scrollTop); finish(); }); }} onKeyDown={event => {
            let next = selected();
            if (event.key === 'ArrowDown')
                next++;
            else if (event.key === 'ArrowUp')
                next--;
            else if (event.key === 'Home')
                next = 0;
            else if (event.key === 'End')
                next = items.length - 1;
            else if (event.key === 'PageDown')
                next += Math.max(1, range().end - range().start - 12);
            else if (event.key === 'PageUp')
                next -= Math.max(1, range().end - range().start - 12);
            else
                return;
            event.preventDefault();
            next = Math.max(0, Math.min(items.length - 1, next));
            setActive(next);
            if(items[next])props.onActive?.(items[next]);
            scrollToIndex(next);
        }}><div class="virtual-content" style={{ height: (revision(), positions.total + 'px') }}><For each={keys()}>{key => {
            let row!: HTMLDivElement;
            const index=createMemo(()=>{revision();return keyIndex.get(key)??0;});
            const item = createMemo(() => { revision(); return items[index()]; });
            const content=createMemo(()=>props.render(item(),index()));
            onMount(() => {if(!props.fixedHeights)observer?.observe(row);});
            onCleanup(() => observer?.unobserve(row));
            return <div ref={row} id={'virtual-row-' + key} class="virtual-row" role="option" aria-selected={index() === selected()} aria-setsize={(revision(),items.length)} aria-posinset={index() + 1} data-index={index()} data-key={key} style={{ transform: `translateY(${(revision(), positions.offset(index()))}px)` }}>{content()}</div>;
        }}</For></div></div>;
}
