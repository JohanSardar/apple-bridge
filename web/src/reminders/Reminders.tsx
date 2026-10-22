import { createEffect, createMemo, createSignal, For, onCleanup, onMount, on, Show } from 'solid-js';
import { AppMenu } from '../app/AppMenu';
import { registerShortcut } from '../app/keyboard';
import { router } from '../app/router';
import { auth } from '../auth/session';
import { Icon } from '../design/Icon';
import { enterPane, reducedMotion, springAnimation } from '../motion/animate';
import { swipeBack } from '../motion/gestures';
import { VirtualList } from '../virtual/VirtualList';
import { mediaSource } from '../messages/media/source';
import { dueLabel, sectionLabel, object, overdue, smartNames, text, useRows, useView, viewerZone, type Item, type Items, type List, type Lists, type Row } from './store';
import './reminders.css';
import { reminderStrings as s, priorities, reminderCount, priorityLabel, earlyLabel, subtaskLabel, disclosureLabel, incompleteCount, completedCount } from './strings';
const rowKey = (row: Row) => row.item?.reminder.id ?? 'section:' + row.section!.id;
const tileColors: Record<string, string> = { today: '#2785e9', scheduled: '#bd2828', all: '#555555', flagged: '#bb6400', completed: '#58616b', assigned: '#277c30' };
function routeParts() { return router.route().path.split('/').filter(Boolean).map(part => { try { return decodeURIComponent(part); } catch { return part; } }); }
function safeURL(value: unknown) { try { const url = new URL(text(value)); return ['http:', 'https:'].includes(url.protocol) ? url.href : undefined; } catch { return; } }
export default function Reminders() {
    const routeSelector = createMemo(() => routeParts()[1] ?? ''), detailID = createMemo(() => routeParts()[2]);
    const [selector, setSelector] = createSignal(routeSelector());
    const lists = useView<Lists>('reminders.lists', () => ({ time_zone: viewerZone }));
    const [completed, setCompleted] = createSignal(false), [sort, setSort] = createSignal('manual'), [search, setSearch] = createSignal(''), [collapsed, setCollapsed] = createSignal(new Set<string>());
    const compactMedia = matchMedia('(max-width:1023px)'), [compact, setCompact] = createSignal(compactMedia.matches);
    const [phase, setPhase] = createSignal<'home' | 'push' | 'list' | 'back' | 'swipe'>(selector() ? 'list' : 'home');
    const selectedList = createMemo(() => lists().lists?.find(entry => entry.list.id === selector()));
    const title = createMemo(() => search().trim() ? s.searchResults : selector().startsWith('smart:') ? smartNames[selector().slice(6)] ?? s.app : selector().startsWith('tag:') ? '#' + selector().slice(4) : text(selectedList()?.list.data.title) || s.app);
    const [pendingFocus, setPendingFocus] = createSignal<string>();
    const [closedSubtasks, setClosedSubtasks] = createSignal(new Set<string>());
    const items = useView<Items>('reminders.items', () => {
        const selected = selector(), query: Record<string, unknown> = { time_zone: viewerZone, show_completed: completed(), sort: sort() };
        if (search().trim()) return { ...query, smart: 'all', show_completed: true };
        if (!selected) return;
        if (selected.startsWith('smart:')) query.smart = selected.slice(6);
        else if (selected.startsWith('tag:')) query.tag = selected.slice(4);
        else query.list = selected;
        return query;
    });
    const rows = useRows(items, search, closedSubtasks);
    const [activeKey, setActiveKey] = createSignal<string>();
    const active = createMemo(() => { const key = activeKey(); return key ? rows().find(row => rowKey(row) === key)?.item ?? undefined : undefined; });
    const keyboardItem = () => activeKey() ? active() : rows().find(row => row.item)?.item ?? undefined;
    const knownCount = createMemo(() => {
        if (search().trim()) return items().sections ? rows().filter(row => row.item).length : undefined;
        if (selectedList()) return selectedList()?.count;
        if (selector().startsWith('smart:')) return lists().smart?.find(tile => tile.id === selector().slice(6))?.count;
        if (selector().startsWith('tag:')) return lists().tags?.find(tag => tag.name === selector().slice(4))?.count;
        return items().count;
    });
    const countText = () => {
        const count = knownCount(); if (count === undefined) return '';
        if (search().trim()) return reminderCount(count);
        if (selector() === 'smart:completed') return completedCount(count);
        const fromSidebar = selectedList()?.count !== undefined || (selector().startsWith('smart:') && lists().smart?.some(tile => tile.id === selector().slice(6) && tile.count !== undefined)) || (selector().startsWith('tag:') && lists().tags?.some(tag => tag.name === selector().slice(4)));
        return fromSidebar ? incompleteCount(count) : reminderCount(count);
    };
    const selectedItem = createMemo(() => {
        const id = detailID(); if (!id) return;
        const pending = (items().sections ?? []).flatMap(group => group.items);
        while (pending.length) { const item = pending.pop()!; if (item.reminder.id === id) return item; pending.push(...item.subtasks); }
    });
    const listByID = createMemo(() => new Map((lists().lists ?? []).map(entry => [entry.list.id, entry])));
    const tileByID = createMemo(() => new Map((lists().smart ?? []).map(tile => [tile.id, tile])));
    const tagByName = createMemo(() => new Map((lists().tags ?? []).map(tag => [tag.name, tag])));
    const hasSelection = () => Boolean(selector() || search().trim());
    let sidebar!: HTMLElement, content!: HTMLElement, cancelEnter: (() => void) | undefined, motionGeneration = 0, swipeCommitted = false;
    const clearMotion = () => { content.style.transform = ''; content.style.opacity = ''; sidebar.style.transform = ''; sidebar.style.opacity = ''; };
    const focusContent = () => {
        const requested = selector();
        queueMicrotask(() => {
            if (!content.isConnected || phase() !== 'list' || selector() !== requested) return;
            const target = content.querySelector<HTMLElement>(search().trim() ? '.reminders-search input' : '.virtual-viewport');
            (target ?? content).focus({ preventScroll: true }); setPendingFocus(target ? undefined : requested);
        });
    };
    function finishBack(previous: string) {
        setPendingFocus(undefined); setSelector(''); setSearch(''); setActiveKey(undefined); setPhase('home'); clearMotion();
        queueMicrotask(() => { if (sidebar.isConnected && phase() === 'home') (sidebar.querySelector<HTMLElement>('[data-selector="' + CSS.escape(previous) + '"]:not(:disabled)') ?? sidebar.querySelector<HTMLElement>('input,button:not(:disabled)'))?.focus({ preventScroll: true }); });
    }
    function transition(forward: boolean, finished: () => void) {
        const token = ++motionGeneration, interrupted = phase() === 'push' || phase() === 'back' || phase() === 'swipe';
        const direction = getComputedStyle(content).direction === 'rtl' ? -1 : 1;
        const offset = interrupted ? new DOMMatrix(getComputedStyle(content).transform).m41 * direction / Math.max(1, content.clientWidth) : forward ? 1 : 0;
        cancelEnter?.(); setPhase(forward ? 'push' : 'back');
        if (reducedMotion()) { finished(); return; }
        queueMicrotask(() => {
            if (token !== motionGeneration || !content.isConnected) return;
            const width = content.clientWidth;
            const animation = springAnimation(value => { content.style.transform = `translateX(${value * width * direction}px)`; sidebar.style.transform = `translateX(${(value - 1) * width * .3 * direction}px)`; sidebar.style.opacity = String(.7 + value * .3); }, offset, forward ? 0 : 1, { response: .35, damping: 1 }, 0, () => { if (token === motionGeneration) finished(); });
            cancelEnter = () => animation.cancel();
        });
    }
    const navigate = (value: string) => { setSearch(''); setActiveKey(undefined); router.navigate('/reminders/' + encodeURIComponent(value)); };
    const closeDetails = () => { router.navigate('/reminders/' + encodeURIComponent(selector()), true); queueMicrotask(() => content.querySelector<HTMLElement>('.virtual-viewport')?.focus({ preventScroll: true })); };
    const backToLists = () => { if (routeSelector()) router.navigate('/reminders'); };
    const openDetails = (item?: Item) => { if (item) { setActiveKey(item.reminder.id); router.navigate('/reminders/' + encodeURIComponent(selector() || 'smart:all') + '/' + encodeURIComponent(item.reminder.id)); } };
    onMount(() => {
        const changed = () => setCompact(compactMedia.matches);
        compactMedia.addEventListener('change', changed);
        const shortcuts = [registerShortcut({ key: 'f', input: true, action: () => document.querySelector<HTMLInputElement>('.reminders-search input')?.focus() }), registerShortcut({ key: '[', action: () => router.back() }), registerShortcut({ key: 'i', action: () => openDetails(keyboardItem()) }), ...Object.keys(smartNames).map((id, index) => registerShortcut({ key: String(index + 1), action: () => navigate('smart:' + id) }))];
        const dispose = swipeBack(content, sidebar, () => compact() && phase() === 'list' && !detailID(), () => { swipeCommitted = true; backToLists(); }, () => { motionGeneration++; cancelEnter?.(); setPhase('swipe'); }, () => { if (routeSelector()) { setPhase('list'); focusContent(); } });
        onCleanup(() => { dispose(); shortcuts.forEach(dispose => dispose()); compactMedia.removeEventListener('change', changed); cancelEnter?.(); motionGeneration++; });
    });
    createEffect(on(routeSelector, next => {
        if (!content) return;
        const previous = selector();
        if (next !== previous) setActiveKey(undefined);
        if (!compact()) { cancelEnter?.(); motionGeneration++; setSelector(next); setPhase(next ? 'list' : 'home'); if (next) cancelEnter = enterPane(content, false, 1, 0); else finishBack(previous); return; }
        if (next) { setSelector(next); transition(true, () => { clearMotion(); setPhase('list'); focusContent(); }); }
        else if (swipeCommitted) { swipeCommitted = false; motionGeneration++; finishBack(previous); }
        else if (previous) transition(false, () => finishBack(previous));
        else { setPhase('home'); clearMotion(); }
    }));
    createEffect(() => {
        const pending = pendingFocus(), ready = items().sections, current = selector(), settled = phase();
        if (pending && pending === current && settled === 'list' && ready) {
            setPendingFocus(undefined);
            if (document.activeElement === content && rows().length) focusContent();
        }
    });
    function toggleSubtasks(item: Item, open?: boolean) {
        setActiveKey(item.reminder.id);
        setClosedSubtasks(previous => { const next = new Set(previous), expanded = open ?? next.has(item.reminder.id); if (expanded) next.delete(item.reminder.id); else next.add(item.reminder.id); return next; });
    }
    function groupHidden(entry: List) {
        const seen = new Set<string>(); let parent = entry.group_id;
        while (parent && !seen.has(parent)) { if (collapsed().has(parent)) return true; seen.add(parent); parent = listByID().get(parent)?.group_id ?? null; }
        return false;
    }
    function Search() { return <label class="reminders-search"><Icon name="search"/><input type="search" aria-label={s.searchLabel} placeholder={s.search} value={search()} onInput={event => { const value = event.currentTarget.value; setSearch(value); if (value && !selector()) { router.navigate('/reminders/' + encodeURIComponent('smart:all')); queueMicrotask(() => content.querySelector<HTMLInputElement>('.reminders-search input')?.focus()); } }}/></label>; }
    function Highlight(props: { value: string }) {
        const match = createMemo(() => { const needle = search().trim(); return needle ? props.value.toLocaleLowerCase().indexOf(needle.toLocaleLowerCase()) : -1; });
        return <Show when={match() >= 0} fallback={props.value}>{props.value.slice(0, match())}<mark>{props.value.slice(match(), match() + search().trim().length)}</mark>{props.value.slice(match() + search().trim().length)}</Show>;
    }
    function Count(props: { value?: number }) { return <Show when={props.value !== undefined}><span class="reminder-count">{props.value}</span></Show>; }
    function ReminderRow(props: { item: Item; depth: number }) {
        const data = () => props.item.reminder.data;
        const color = () => text(listByID().get(text(data().list_id))?.list.data.color) || '#4b8cf5';
        const tags = () => Array.isArray(data().tags) ? data().tags as string[] : [];
        return <><article class="reminder-row" classList={{ 'reminder-completed': Boolean(data().completed) }} data-depth={props.depth} style={{ '--depth': props.depth, '--list-color': color() }}>
            <span class="reminder-ring" role="img" aria-label={data().completed ? s.completed : s.incomplete}><Show when={data().completed}><Icon name="check"/></Show></span>
            <div class="reminder-copy"><button class="reminder-title-button" tabindex="-1" onClick={() => openDetails(props.item)}><Show when={priorities[Number(data().priority)]}><span class="reminder-priority" aria-label={priorityLabel(Number(data().priority))}>{Number(data().priority) === 1 ? '!!!' : Number(data().priority) === 5 ? '!!' : '!'}</span></Show><span class="reminder-title"><Highlight value={text(data().title)}/></span></button>
                <Show when={data().notes}><p class="reminder-notes"><Highlight value={text(data().notes)}/></p></Show>
                <div class="reminder-metadata"><Show when={props.item.subtasks.length}><button tabindex="-1" class="reminder-subtasks" aria-label={disclosureLabel(text(data().title), !closedSubtasks().has(props.item.reminder.id))} aria-expanded={!closedSubtasks().has(props.item.reminder.id)} onClick={() => toggleSubtasks(props.item)}><span aria-hidden="true">{closedSubtasks().has(props.item.reminder.id) ? '›' : '⌄'}</span>{subtaskLabel(props.item.subtasks.length)}</button></Show><Show when={data().due}><time classList={{ 'reminder-overdue': overdue(data()) }} datetime={text(data().due)}>{dueLabel(data().due)}</time></Show><Show when={Array.isArray(data().recurrence) && (data().recurrence as unknown[]).length}><span aria-label={s.repeats}>↻</span></Show><Show when={Array.isArray(data().alarms) && (data().alarms as unknown[]).some(value => object(value).location)}><span aria-label={s.location}>⌖</span></Show><For each={tags()}>{tag => <span class="reminder-tag">#{tag}</span>}</For><Show when={data().assignment}><span class="reminder-assignee"><span class="reminder-assignee-avatar" aria-hidden="true"><Icon name="contacts"/></span>{object(object(data().assignment).assignee).is_me === true ? s.assignedMe : s.assigned}</span></Show></div>
                <Show when={safeURL(data().url)}>{url => <a class="reminder-url" href={url()} target="_blank" rel="noopener noreferrer">{new URL(url()).hostname}</a>}</Show>
                <Show when={props.item.attachments.length}><div class="reminder-attachments"><For each={props.item.attachments}>{attachment => <Show when={mediaSource(attachment, 'thumb')} fallback={<span>{text(attachment.data.filename) || s.attachment}</span>}>{source => <a href={mediaSource(attachment, 'original')} target="_blank" rel="noopener noreferrer"><img src={source()} alt={text(attachment.data.filename) || s.image} loading="lazy" width="100" height="80"/></a>}</Show>}</For></div></Show>
            </div><Show when={data().flagged === true}><span class="reminder-flag" role="img" aria-label="Flagged">⚑</span></Show>
        </article></>;
    }
    function Details(props: { item: Item }) {
        const data = () => props.item.reminder.data;
        function Body() { return <><header><h2>{s.details}</h2><button class="icon-button" autofocus aria-label={s.closeDetails} onClick={closeDetails}><Icon name="close"/></button></header><h3>{text(data().title)}</h3><Show when={data().notes}><p>{text(data().notes)}</p></Show><dl><dt>{s.list}</dt><dd>{text(listByID().get(text(data().list_id))?.list.data.title)}</dd><Show when={data().due}><dt>{s.due}</dt><dd>{dueLabel(data().due)}</dd></Show><Show when={typeof data().priority === 'number'}><dt>{s.priority}</dt><dd>{Number(data().priority) === 0 ? s.none : priorities[Number(data().priority)] ?? String(data().priority)}</dd></Show><Show when={data().flagged === true}><dt>{s.flag}</dt><dd>{s.flagged}</dd></Show><Show when={data().completed_at}><dt>{s.completed}</dt><dd>{dueLabel(data().completed_at)}</dd></Show><Show when={Array.isArray(data().early_reminders) && (data().early_reminders as unknown[]).length}><dt>{s.early}</dt><dd>{(data().early_reminders as number[]).map(delta => earlyLabel(delta)).join(', ')}</dd></Show><Show when={data().assignment}><dt>{s.assignedTo}</dt><dd>{object(object(data().assignment).assignee).is_me === true ? s.me : object(object(data().assignment).assignee).is_me === false ? s.other : s.unknownIdentity}</dd></Show></dl><Show when={safeURL(data().url)}>{url => <a href={url()} target="_blank" rel="noopener noreferrer">{url()}</a>}</Show><p class="reminder-read-only">{s.readOnly}</p></>; }
        return <Show when={compact()} fallback={<aside class="reminder-details-panel" aria-label={s.detailsLabel}><Body/></aside>}><dialog ref={element => queueMicrotask(() => element.showModal())} class="reminder-details" aria-label={s.detailsLabel} onClose={closeDetails}><Body/></dialog></Show>;
    }
    return <div class="app-shell reminders-app" classList={{ 'reminders-selected': hasSelection(), 'reminders-details-open': Boolean(selectedItem()), 'reminders-show-sidebar': phase() !== 'list', 'reminders-show-content': phase() !== 'home' }} data-phase={phase()}>
        <aside ref={sidebar} class="sidebar reminders-sidebar" inert={compact() && phase() !== 'home'} aria-hidden={compact() && phase() !== 'home' ? true : undefined}>
            <header class="toolbar"><h1>{s.app}</h1></header><div class="reminders-sidebar-body" data-scroll="reminders-sidebar">
                <Show when={!compact() || phase() !== 'list'}><Search/></Show>
                <Show when={auth.live()?.status() === 'offline'}><p class="offline-banner" role="status">{s.offline}</p></Show>
                <Show when={lists().error}><p role="alert">{s.listError}</p></Show>
                <nav class="reminder-tiles" aria-label={s.smartLists}><For each={lists().smart?.map(tile => tile.id)}>{id => <Show when={tileByID().get(id)}>{tile => <button data-smart={tile().id} data-selector={'smart:' + tile().id} class={'reminder-tile tile-' + tile().id} aria-label={smartNames[tile().id]} aria-pressed={selector() === 'smart:' + tile().id} disabled={tile().status === 'unsupported'} onClick={() => navigate('smart:' + tile().id)}><span class="reminder-tile-glyph" style={{ 'background-color': tileColors[tile().id] }} aria-hidden="true"><Show when={tile().id === 'flagged'} fallback={<Icon name={tile().id === 'assigned' ? 'contacts' : tile().id === 'completed' ? 'check' : tile().id === 'all' ? 'list' : 'calendar'}/>}>⚑</Show></span><Count value={tile().count}/><span>{smartNames[tile().id]}</span><Show when={tile().status === 'unsupported'}><small>{s.unavailable}</small></Show></button>}</Show>}</For><For each={lists().lists?.filter(entry => entry.pinned === true).map(entry => entry.list.id)}>{id => <Show when={listByID().get(id)}>{entry => <button class="reminder-tile reminder-pinned-tile" data-selector={id} aria-label={text(entry().list.data.title)} aria-pressed={selector() === id} disabled={entry().status === 'unsupported'} onClick={() => navigate(id)}><span class="reminder-tile-glyph" style={{ 'background-color': text(entry().list.data.color) || '#4b8cf5' }} aria-hidden="true">{text(entry().list.data.badge_emblem) || '☷'}</span><Count value={entry().count}/><span>{text(entry().list.data.title)}</span><Show when={entry().status === 'unsupported'}><small>{s.unavailable}</small></Show></button>}</Show>}</For></nav>
                <h2 class="reminder-sidebar-title">{s.myLists}</h2><nav class="reminder-lists" aria-label={s.myLists}><For each={lists().lists?.filter(entry => entry.pinned !== true).map(entry => entry.list.id)}>{id => <Show when={listByID().get(id)}>{entry => <Show when={!groupHidden(entry())}><button class="reminder-list" data-selector={entry().list.id} classList={{ selected: selector() === entry().list.id, 'reminder-group': entry().list.data.is_group === true, 'reminder-group-child': Boolean(entry().group_id) }} aria-label={text(entry().list.data.title)} aria-expanded={entry().list.data.is_group === true ? !collapsed().has(entry().list.id) : undefined} disabled={entry().status === 'unsupported'} aria-describedby={entry().status === 'unsupported' ? 'reminder-unsupported-' + entry().list.id : undefined} onClick={() => {
                    if (entry().list.data.is_group === true) setCollapsed(previous => { const next = new Set(previous); if (next.has(entry().list.id)) next.delete(entry().list.id); else next.add(entry().list.id); return next; });
                    else navigate(entry().list.id);
                }}><span class="reminder-emblem" style={{ 'background-color': entry().list.data.is_group === true ? 'transparent' : text(entry().list.data.color) || '#4b8cf5' }} aria-hidden="true">{entry().list.data.is_group === true ? collapsed().has(entry().list.id) ? '›' : '⌄' : text(entry().list.data.badge_emblem) || '☷'}</span><span class="reminder-list-name">{text(entry().list.data.title)}</span><Show when={entry().status !== 'unsupported'}><Count value={entry().count}/></Show><Show when={entry().status === 'unsupported'}><span id={'reminder-unsupported-' + entry().list.id} class="reminder-unsupported" title={s.unsupportedFilterHelp} aria-label={s.unsupportedFilter}>{s.unavailable}</span></Show></button></Show>}</Show>}</For></nav>
                <Show when={lists().tags?.length}><h2 class="reminder-sidebar-title">{s.tags}</h2><nav class="reminder-tags" aria-label={s.tags}><For each={lists().tags?.map(tag => tag.name)}>{name => <Show when={tagByName().get(name)}>{tag => <button data-selector={'tag:' + tag().name} aria-label={'#' + tag().name} aria-pressed={selector() === 'tag:' + tag().name} onClick={() => navigate('tag:' + tag().name)}>#{tag().name}<Count value={tag().count}/></button>}</Show>}</For></nav></Show>
            </div><footer class="sidebar-footer"><AppMenu/></footer>
        </aside><main ref={content} class="reminders-content" tabindex="-1" inert={compact() && phase() === 'home'} aria-hidden={compact() && phase() === 'home' ? true : undefined}>
            <Show when={hasSelection()} fallback={<div class="empty quiet-empty"><p>{s.selectList}</p></div>}>
                <header class="reminders-toolbar toolbar"><button class="reminders-back icon-button" aria-label={s.back} onClick={backToLists}><Icon name="chevron"/></button><Show when={compact()}><Search/></Show><div class="reminders-controls"><label><input type="checkbox" checked={completed()} onChange={event => setCompleted(event.currentTarget.checked)}/>{s.showCompleted}</label><label class="reminder-sort">{s.sort}<select aria-label={s.sort} value={sort()} onChange={event => setSort(event.currentTarget.value)}><option value="manual">{s.manual}</option><option value="due">{s.dueDate}</option><option value="created">{s.created}</option><option value="priority">{s.priority}</option><option value="title">{s.title}</option></select></label></div></header>
                <div class="reminders-heading" style={{ '--list-color': text(selectedList()?.list.data.color) || tileColors[selector().slice(6)] || '#4b8cf5' }}><h1>{title()}</h1><Show when={knownCount() !== undefined}><p>{countText()}</p></Show></div>
                <Show when={items().error} fallback={<Show when={items().sections} fallback={<p class="reminders-message" role="status">{s.loading}</p>}><Show when={rows().length} fallback={<p class="reminders-message">{s.empty}</p>}><div class="reminders-virtual" onKeyDown={event => { const selected = keyboardItem(); if (event.target === event.currentTarget.querySelector('.virtual-viewport') && (event.key === 'ArrowLeft' || event.key === 'ArrowRight') && selected?.subtasks.length) { event.preventDefault(); toggleSubtasks(selected, event.key === 'ArrowRight'); } if (event.target === event.currentTarget.querySelector('.virtual-viewport') && (event.key === 'Enter' || event.key === ' ')) { event.preventDefault(); openDetails(keyboardItem()); } }}><VirtualList items={rows()} keyFor={rowKey} estimate={row => 70 + (row.section ? 44 : 0)} label={s.app} selectedKey={activeKey() ?? detailID()} windowKey={selector() + search()} onActive={row => setActiveKey(rowKey(row))} render={row => <><Show when={row.section}><h2 class="reminder-section-title">{sectionLabel(row.section!)}</h2></Show><Show when={row.item} keyed>{item => <ReminderRow item={item} depth={row.depth}/>}</Show></>}/></div></Show></Show>}><div class="reminders-message" role="alert"><p>{items().error === 'capacity' ? s.capacity : s.itemsError}</p><Show when={knownCount() !== undefined}><p>{countText()}</p></Show><Show when={items().error === 'capacity'}><p>{s.capacityHelp}</p></Show></div></Show>
            </Show>
        </main><Show when={selectedItem()}>{item => <Details item={item()}/>}</Show>
    </div>;
}
