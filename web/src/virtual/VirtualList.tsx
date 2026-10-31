import { batch, createEffect, createMemo, createSignal, createUniqueId, For, onCleanup, onMount, untrack, type JSX } from 'solid-js';
import { beginWork } from '../motion/work';
import { Positions } from './positions';
import { viewportHeight } from '../app/viewport';
export type VirtualListHandle = {
    scrollToIndex: (index: number, align?: 'start' | 'end') => void;
    anchor: () => {
        key: string;
        offset: number;
    } | undefined;
};
type VirtualRenderer<T>={render:(item:T,index:number)=>JSX.Element;renderReactive?:(item:()=>T,index:()=>number)=>JSX.Element}|{render?:never;renderReactive:(item:()=>T,index:()=>number)=>JSX.Element};
export function VirtualList<T>(props: {
    items: readonly T[];
    keyFor: (item: T) => string;
    estimate?: (item: T) => number;
    bottomAnchored?: boolean;
    fixedHeights?: boolean;
    initialKey?: string;
    initialAlign?: 'start'|'end';
    initialPrefetch?: boolean;
    initialHeight?: number;
    initialBatchSize?: number;
    overscan?: number;
    selectedKey?: string;
    selectedKeys?: ReadonlySet<string>;
    keepTop?: boolean;
    windowKey?: string;
    measurementKey?: string;
    onActive?: (item:T,event:KeyboardEvent)=>void;
    onBottom?: (value:boolean)=>void;
    label: string;
    ref?: (handle: VirtualListHandle) => void;
    onVisible?: (start: number, end: number) => void;
} & VirtualRenderer<T>) {
    const rowPrefix=createUniqueId()+'-virtual-row-',initialKey=props.initialKey,initialAlign=props.initialAlign;
    const controlled='selectedKey' in props;
    const selected=()=>{revision();return controlled?(props.selectedKey?keyIndex.get(props.selectedKey)??-1:-1):active();};
    let viewport!: HTMLDivElement;
    let content!:HTMLDivElement;
    let positions = new Positions([]), items: readonly T[] = [];
    let keyIndex = new Map<string, number>();
    const initialIndex=()=>keyIndex.get(initialKey!)??0;
    let firstMountAnchor:boolean|undefined=props.initialPrefetch?true:undefined;
    let previousWindowKey=props.windowKey;
    let previousMeasurementKey=props.measurementKey;
    const cache = new Map<string,{height:number;estimate:number|undefined}>();
    const [revision, setRevision] = createSignal(0), [scroll, setScroll] = createSignal(0), [height, setHeight] = createSignal(Math.max(1,props.initialHeight??viewportHeight()-140)), [active, setActive] = createSignal(0);
    let frame = 0, initialFrame = 0, scrollResidual=0, assignedScroll:number|undefined;
    const batchSize = Math.max(1, props.initialBatchSize ?? Infinity), [initialCount, setInitialCount] = createSignal(batchSize);
    const setScrollPosition=(top:number,preserve=false)=>{
        if(assignedScroll!==undefined&&Math.abs(viewport.scrollTop-assignedScroll)>.01)scrollResidual=0;
        const desired=top+(preserve?scrollResidual:0);viewport.scrollTop=desired;let actual=viewport.scrollTop;
        if(Math.abs(desired-actual)>.5&&Math.abs(desired-actual)<1){viewport.scrollTop=Math.round(desired);actual=viewport.scrollTop;}
        scrollResidual=preserve&&desired>=0&&desired<=viewport.scrollHeight-viewport.clientHeight&&Math.abs(desired-actual)<1?desired-actual:0;assignedScroll=actual;
    };
    const range = createMemo(() => { revision(); const top = scroll(),padding=props.overscan??6; const start = Math.max(0, positions.indexAt(top) - padding), end = Math.min(items.length, positions.indexAt(top + height()) + padding+1); return { start, end }; },undefined,{equals:(a,b)=>a.start===b.start&&a.end===b.end});
    const anchor = () => { if (!items.length)
        return; const index = positions.indexAt(viewport.scrollTop); return { key: props.keyFor(items[index]), offset: viewport.scrollTop - positions.offset(index) }; };
    const refresh = (top=viewport.scrollTop) => batch(() => { setScroll(top); setRevision(value => value + 1); });
    const bottom=()=>{setScrollPosition(viewport.scrollHeight);refresh();};
    const scrollToIndex = (index: number, align: 'start' | 'end' = 'start',initial=false) => {
        if(!initial&&mounting&&index===initialIndex()&&align===initialAlign)return;
        const top=align==='end'?positions.offset(index+1)-(initial?height():viewport.clientHeight):positions.offset(index);
        if(initial)refresh(Math.max(0,Math.min(top,positions.total-height())));
        else{mounting=false;setScrollPosition(top);refresh();if(align==='end'&&index===items.length-1)queueMicrotask(bottom);}
    };
    const cancelFirstMountAnchor=(event:Event)=>{if(firstMountAnchor&&event.isTrusted&&Math.abs(viewport.scrollTop-(assignedScroll??scroll()))>.5)firstMountAnchor=false;};
    createEffect(() => {
        const next = props.items;
        const userTop=firstMountAnchor===false?viewport.scrollTop:undefined;
        const finish=beginWork('virtual-items');
        const shifted=props.windowKey!==previousWindowKey;previousWindowKey=props.windowKey;
        const first = items.length === 0||mounting||firstMountAnchor===true&&(!controlled||untrack(selected)===initialIndex());
        if(!props.initialPrefetch)firstMountAnchor=undefined;
        const previous = !first&&userTop===undefined ? anchor() : undefined;
        const atTop=!first ? viewport.scrollTop<=2 : true;
        const atBottom = !first ? viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight <= 2 : false;
        items = next;
        keyIndex = new Map(items.map((item, index) => [props.keyFor(item), index]));
        const invalidated=props.measurementKey!==previousMeasurementKey;
        if(invalidated){cache.clear();previousMeasurementKey=props.measurementKey;}
        for(const key of cache.keys())if(!keyIndex.has(key))cache.delete(key);
        const remeasure=new Set<string>();
        positions = new Positions(items.map(item => {const key=props.keyFor(item),estimated=props.estimate?.(item),cached=cache.get(key);if(cached&&cached.estimate!==estimated)remeasure.add(key);const height=(!props.fixedHeights&&cached?.estimate===estimated?cached?.height:estimated)??60;return Number.isFinite(height)&&height>0?height:60;}));
        if(invalidated||remeasure.size)viewport.querySelectorAll<HTMLElement>('.virtual-row').forEach(row=>{if(invalidated||remeasure.has(row.dataset.key!)){observer.unobserve(row);observer.observe(row);}});
        content.style.height=positions.total+'px';
        if(userTop!==undefined&&!(props.bottomAnchored&&atBottom)){mounting=false;refresh(userTop);finish();return;}
        if(first&&initialKey!=null){scrollToIndex(initialIndex(),initialAlign,true);if(!mounting)viewport.scrollTop=assignedScroll=scroll();}
        else {
            if (props.bottomAnchored && (atBottom || first) && (!shifted || first)) {
                setScrollPosition(Math.max(0,positions.total-viewport.clientHeight));
                queueMicrotask(bottom);
            }
            else if(props.keepTop&&atTop)setScrollPosition(0);
            else if (previous) {
                const index = (keyIndex.get(previous.key) ?? -1);
                if (index >= 0)
                    setScrollPosition(positions.offset(index)+previous.offset,true);
            }
            refresh();
        }
        finish();
    });
    let mounting=initialKey!=null;
    const observer = new ResizeObserver(entries=>batch(() => {
        const saved = mounting?undefined:anchor(), atBottom = !mounting&&positions.total - viewport.scrollTop - height() <= 2;
        let changed = false;
        for (const entry of entries) {
            if (entry.target === viewport) {
                changed=true;
                setHeight(entry.contentRect.height);
                continue;
            }
            const row = entry.target as HTMLElement, index = Number(row.dataset.index), item = items[index];
            if (!item || row.dataset.key !== props.keyFor(item) || mounting&&(index<range().start||index>=range().end))
                continue;
            const measured = entry.borderBoxSize[0].blockSize;
            cache.set(props.keyFor(item),{height:measured,estimate:props.estimate?.(item)});
            if (Math.abs(positions.height(index) - measured) > .01) {
                positions.measure(index, measured);
                changed = true;
            }
        }
        if (changed) {
            content.style.height=positions.total+'px';
            if(mounting){if(firstMountAnchor===false)refresh(scroll());else{scrollToIndex(initialIndex(),initialAlign,true);viewport.scrollTop=scroll();}mounting=false;return;}
            if (props.bottomAnchored && atBottom) {
                setScrollPosition(Math.max(0,positions.total-viewport.clientHeight));
                queueMicrotask(bottom);
            }
            else if (saved) {
                const index = (keyIndex.get(saved.key) ?? -1);
                if (index >= 0)
                    setScrollPosition(positions.offset(index)+saved.offset,true);
            }
            refresh();
        }
    }));
    onMount(() => { props.ref?.({ scrollToIndex, anchor }); observer.observe(viewport); });
    onCleanup(() => { observer.disconnect(); cancelAnimationFrame(frame); cancelAnimationFrame(initialFrame); });
    createEffect(() => { revision(); const value=range(); props.onVisible?.(value.start,value.end); });
    createEffect(() => {revision();props.onBottom?.(positions.total-scroll()-height()<=2);});
    createEffect(() => {
        const count = initialCount(), visible = range();
        if (!Number.isFinite(count)) return;
        cancelAnimationFrame(initialFrame);
        initialFrame = requestAnimationFrame(() => setInitialCount(count + batchSize >= visible.end - visible.start ? Infinity : count + batchSize));
    });
    const initialBusy = () => initialCount() < range().end - range().start;
    const keys = createMemo(() => {
        revision();
        const visible = range(), count = initialCount(), start = initialAlign === 'end' ? Math.max(visible.start, visible.end - count) : visible.start;
        return items.slice(start, Math.min(visible.end, start + count)).map(props.keyFor);
    });
    const activeDescendant=createMemo(()=>{const item=items[selected()];if(!item)return;const key=props.keyFor(item);return keys().includes(key)?rowPrefix+key:undefined;});
    return <div ref={viewport} class="virtual-viewport" data-scroll={props.label} tabindex="0" role="listbox" aria-busy={initialBusy()} aria-multiselectable={props.selectedKeys ? true : undefined} aria-label={props.label} aria-activedescendant={activeDescendant()} onScroll={event => { cancelFirstMountAnchor(event); if (!frame)
        frame = requestAnimationFrame(() => { frame = 0; const finish=beginWork('virtual-scroll'); setScroll(viewport.scrollTop); finish(); }); }} onKeyDown={event => {
            if (event.target !== event.currentTarget) return;
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
                next += Math.max(1, range().end - range().start - 2*(props.overscan??6));
            else if (event.key === 'PageUp')
                next -= Math.max(1, range().end - range().start - 2*(props.overscan??6));
            else
                return;
            event.preventDefault();
            setInitialCount(Infinity);
            next = Math.max(0, Math.min(items.length - 1, next));
            setActive(next);
            if(items[next])props.onActive?.(items[next],event);
            scrollToIndex(next);
        }}><div ref={content} class="virtual-content"><For each={keys()}>{key => {
            let row!: HTMLDivElement;
            const index=createMemo(()=>{revision();return keyIndex.get(key)??0;});
            const item = createMemo(() => { revision(); return items[index()]; });
            const stableContent=props.renderReactive?.(item,index);
            const content=props.renderReactive?()=>stableContent:createMemo(()=>props.render!(item(),index()));
            onMount(() => {if(!props.fixedHeights)observer.observe(row);});
            onCleanup(() => observer.unobserve(row));
            return <div ref={row} id={rowPrefix + key} class="virtual-row" role="option" aria-selected={props.selectedKeys ? props.selectedKeys.has(key) : index() === selected()} aria-setsize={(revision(),items.length)} aria-posinset={index() + 1} data-index={index()} data-key={key} style={{ transform: `translateY(${(revision(), positions.offset(index()))}px)` }}>{content()}</div>;
        }}</For></div></div>;
}
