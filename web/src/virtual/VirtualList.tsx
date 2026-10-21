import { createEffect, createMemo, createSignal, For, onCleanup, onMount, type JSX } from 'solid-js';
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
    label: string;
    ref?: (handle: VirtualListHandle) => void;
    onVisible?: (start: number, end: number) => void;
}) {
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
        const previous = viewport ? anchor() : undefined;
        const atBottom = viewport ? viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight <= 2 : false;
        const first = items.length === 0;
        items = next;
        keyIndex = new Map(items.map((item, index) => [props.keyFor(item), index]));
        positions = new Positions(items.map(item => { const estimate=cache.get(props.keyFor(item)) ?? props.estimate?.(item) ?? 60; return Number.isFinite(estimate)&&estimate>0?estimate:60; }));
        if (viewport) {
            if (props.bottomAnchored && (atBottom || first)) {
                viewport.scrollTop = Math.max(0, positions.total - viewport.clientHeight);
                queueMicrotask(() => { viewport.scrollTop = viewport.scrollHeight; refresh(); });
            }
            else if (previous) {
                const index = (keyIndex.get(previous.key) ?? -1);
                if (index >= 0)
                    viewport.scrollTop = positions.offset(index) + previous.offset;
            }
            refresh();
        }
        else
            setRevision(value => value + 1);
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
        viewport.querySelectorAll('[data-index]').forEach(element => observer?.observe(element));
        if (props.bottomAnchored)
            scrollToIndex(items.length - 1, 'end');
        refresh();
    });
    onCleanup(() => { observer?.disconnect(); cancelAnimationFrame(frame); });
    createEffect(() => { const value = range(); props.onVisible?.(value.start, value.end); });
    const indices = createMemo(() => Array.from({ length: Math.max(0, range().end - range().start) }, (_, i) => range().start + i));
    return <div ref={viewport} class="virtual-viewport" data-scroll={props.label} tabindex="0" role="listbox" aria-label={props.label} aria-activedescendant={items.length ? 'virtual-row-' + props.keyFor(items[active()]) : undefined} onScroll={() => { if (!frame)
        frame = requestAnimationFrame(() => { frame = 0; setScroll(viewport.scrollTop); }); }} onKeyDown={event => {
            let next = active();
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
            scrollToIndex(next);
        }}><div class="virtual-content" style={{ height: (revision(), positions.total + 'px') }}><For each={indices()}>{index => {
            let row!: HTMLDivElement;
            const item = createMemo(() => { revision(); return items[index]; });
            onMount(() => observer?.observe(row));
            onCleanup(() => observer?.unobserve(row));
            return <div ref={row} id={'virtual-row-' + props.keyFor(item())} class="virtual-row" role="option" aria-selected={index === active()} aria-setsize={(revision(),items.length)} aria-posinset={index + 1} data-index={index} data-key={props.keyFor(item())} style={{ transform: `translateY(${(revision(), positions.offset(index))}px)` }}>{props.render(item(), index)}</div>;
        }}</For></div></div>;
}
