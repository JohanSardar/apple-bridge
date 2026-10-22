import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { AppMenu } from '../app/AppMenu';
import { router } from '../app/router';
import { registerShortcut } from '../app/keyboard';
import { auth } from '../auth/session';
import { enterPane, reducedMotion, sharedElement, springAnimation } from '../motion/animate';
import { VirtualList, type VirtualListHandle } from '../virtual/VirtualList';
import { Card, ContactAvatar } from './Card';
import { ContactEditor } from './Editor';
import { createContactCommands } from './commands';
import { object, searchable, text, useContactPreferences, useContacts, type Item } from './store';
import { s } from './strings';
import { downloadContacts } from './vcard';
import type { BridgeRecord } from '../data/types';
import './contacts.css';
const routeParts = () => new URL(router.route().path,location.origin).pathname.split('/').filter(Boolean).map(part => { try { return decodeURIComponent(part); } catch { return part; } });
export default function Contacts() {
    const preferences = useContactPreferences();
    const commands = createContactCommands(auth.live()!);
    const [editorRevision,setEditorRevision] = createSignal(1);
    const [editing,setEditing] = createSignal(false), [editorItem,setEditorItem] = createSignal<Item>(), [recovery,setRecovery] = createSignal<Record<string,unknown>>(), [deleting,setDeleting] = createSignal<Item[]>(), [lastCommand,setLastCommand] = createSignal('');
    let lastArgs: Record<string,unknown>={}, lastBefore: Item|undefined, editingRoute='';
    const [selecting,setSelecting] = createSignal(false), [checked,setChecked] = createSignal(new Set<string>());
    const selectedContacts = () => entries().filter(item => checked().has(item.contact.id));
    function toggleSelection(item: Item) { setChecked(before=>{const next=new Set(before);if(next.has(item.contact.id))next.delete(item.contact.id);else if(next.size<32)next.add(item.contact.id);return next;}); }
    const [group,setGroup] = createSignal('all'), [search,setSearch] = createSignal(''), [active,setActive] = createSignal(''), [bubble,setBubble] = createSignal(''), [retry,setRetry] = createSignal(0);
    const compactMedia = matchMedia('(max-width:1023px)'), [compact,setCompact] = createSignal(compactMedia.matches);
    const isNew = () => routeParts()[1] === 'new';
    const id = createMemo(() => !['list','new'].includes(routeParts()[1]??'') ? routeParts()[1] : undefined);
    const pane = () => id() || isNew() ? 'card' : routeParts()[1] === 'list' ? 'contacts' : 'lists';
    createEffect(() => { if (routeParts()[1] === 'list') { setGroup(routeParts()[2] || 'all'); setSearch(''); } });
    const captured = useContacts(() => { retry(); return { ...(group() === 'all' || group().startsWith('account:') ? {} : { group: group() }), sort: preferences.sort(), display_order: preferences.display() }; });
    const page = commands.project(captured,preferences.sort,preferences.display,()=>group()==='all');
    const entries = createMemo(() => (page().sections ?? []).flatMap(section => section.contacts).filter(item => !group().startsWith('account:') || item.contact.data.container_id === page().containers?.find(container=>container.id===group().slice(8))?.data.identifier));
    const selected = createMemo(() => entries().find(entry => entry.contact.id === id()));
    const me = createMemo(() => entries().find(entry => entry.is_me === true));
    const rows = createMemo(() => (page().sections ?? []).flatMap(section => {
        const visible = new Set(entries().map(item => item.contact.id));
        const items = section.contacts.filter(item => visible.has(item.contact.id) && (!search().trim() || searchable(item).includes(search().trim().toLocaleLowerCase())) && (search().trim() || item.is_me !== true));
        return items.map((item,index) => ({ item, section: section.key, first: index === 0 }));
    }));
    const title = () => group() === 'all' ? s.all : group().startsWith('account:') ? text(page().containers?.find(entry => entry.id === group().slice(8))?.data.name) || s.app : text(page().groups?.find(entry => entry.group.id === group())?.group.data.name) || s.app;
    const letters = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ#'.split('');
    let list: VirtualListHandle | undefined, card!: HTMLElement, searchField!: HTMLInputElement;
    let cancelPane: (() => void) | undefined, cancelAvatar: (() => void) | undefined, cancelEdit: (() => void) | undefined;
    const goList = (value: string) => { setSelecting(false);setChecked(new Set<string>());setEditing(false); setSearch(''); setActive(''); router.navigate('/contacts/list/' + encodeURIComponent(value)); };
    const back = () => { setEditing(false);router.navigate(id() || isNew() ? '/contacts/list/' + encodeURIComponent(group()) : '/contacts'); };
    const open = (item: Item, source?: Element | null) => {
        if(selecting()){toggleSelection(item);return;}
        setEditing(false);cancelAvatar?.();
        const before = source?.getBoundingClientRect(), clone = source?.cloneNode(true) as HTMLElement | undefined;
        if (clone && before && !reducedMotion()) { clone.setAttribute('aria-hidden','true'); Object.assign(clone.style,{position:'fixed',left:before.x+'px',top:before.y+'px',width:before.width+'px',height:before.height+'px',zIndex:'100',pointerEvents:'none',transformOrigin:'top left'}); document.body.append(clone); }
        router.navigate('/contacts/' + encodeURIComponent(item.contact.id));
        if (clone) requestAnimationFrame(() => {
            const after = card?.querySelector('.avatar')?.getBoundingClientRect();
            if (!before || !after || reducedMotion()) { clone.remove(); return; }
            const animation = springAnimation(value => { clone.style.transform = `translate(${(after.x-before.x)*value}px,${(after.y-before.y)*value}px) scale(${1+(after.width/before.width-1)*value})`; },0,1,{response:.42,damping:.9},0,() => clone.remove());
            cancelAvatar = () => { animation.cancel(); clone.remove(); };
        });
    };
    const canWrite = (operation: string, item?: Item) => commands.capabilities.allowed(operation) && (!item || item.contact.version>0 && item.contact.data.writable===true && !item.contact.data._pending);
    function edit(item?: Item, recover?: Record<string,unknown>) { cancelEdit?.();const change=()=>{setEditorItem(item);setRecovery(recover);editingRoute=router.route().path;setEditorRevision(value=>value+1);setEditing(true);};if(card)cancelEdit=sharedElement(card,change);else change(); }
    function review() {
        const current=object(commands.state(lastCommand())?.error).current as BridgeRecord|undefined;
        let item=selected();
        if(lastBefore && current?.collection==='contacts.contact' && current.id===lastBefore.contact.id && current.version>(item?.contact.version??0) && !(current as BridgeRecord & {deleted?:boolean}).deleted)item={...(item??lastBefore),contact:current};
        edit(lastBefore?item:undefined,lastArgs);
    }
    function create() { setGroup('all');setSearch('');router.navigate('/contacts/new');edit(); }
    function save(event: Event, args: Record<string,unknown>) {
        const before=editorItem(), operation=before?'contacts.update':'contacts.create';
        const command=commands.issue(event,operation,{...args,...(before?{id:before.contact.id}:{})},before);
        if(!command)return;
        lastArgs=args;lastBefore=before;setLastCommand(command);setEditing(false);
        if(!before)router.navigate('/contacts/pending:'+command,true);
    }
    createEffect(()=>{ if(editing() && editingRoute!==router.route().path)setEditing(false); });
    createEffect(()=>{ const command=commands.state(lastCommand()), pending='pending:'+lastCommand();const contact=command?.observed_record_ids?.find(value=>value.startsWith('con_'));if(contact && id()===pending)router.navigate('/contacts/'+encodeURIComponent(contact),true); });
    const jump = (letter: string) => { const index = rows().findIndex(row => row.section === letter); if (index >= 0) { list?.scrollToIndex(index); setActive(rows()[index].item.contact.id); } setBubble(letter); };
    function scrub(event: PointerEvent) { if (!(event.currentTarget instanceof HTMLElement)) return; const buttons = [...event.currentTarget.querySelectorAll('button')]; const nearest = buttons.reduce((best,button) => { const rect=button.getBoundingClientRect(), distance=Math.abs(event.clientY-(rect.top+rect.height/2)); return distance<best.distance ? {letter:button.textContent??'',distance} : best; },{letter:'',distance:Infinity}); if(nearest.letter)jump(nearest.letter); }
    onMount(() => {
        const resized = () => setCompact(compactMedia.matches); compactMedia.addEventListener('change',resized);
        const disposers = [registerShortcut({key:'f',input:true,action:()=>searchField?.focus()}),registerShortcut({key:'[',action:back})];
        onCleanup(() => { compactMedia.removeEventListener('change',resized); for (const dispose of disposers) dispose(); });
    });
    createEffect(() => { const selectedID = id(); cancelPane?.(); if (selectedID && card) queueMicrotask(() => { if (id() === selectedID) { cancelPane = enterPane(card,compact(),1,card.clientWidth); card.querySelector<HTMLElement>('h1')?.focus({preventScroll:true}); } }); });
    onCleanup(() => { cancelPane?.(); cancelAvatar?.(); cancelEdit?.(); });
    return <main class="contacts-app" data-pane={pane()}>
        <aside class="contacts-sidebar" aria-label={s.lists} aria-hidden={compact() && pane() !== 'lists'}>
            <header><h1>{s.lists}</h1><Show when={canWrite('contacts.create')}><button class="contact-new" aria-label="New Contact" disabled={commands.pending()||commands.uncertain().length>0} onClick={create}>New Contact</button></Show></header><nav aria-label="Contact lists"><button classList={{selected:group()==='all'}} onClick={() => goList('all')}><span>{s.all}</span><Show when={group()==='all' && page().count !== undefined}><span>{page().count}</span></Show></button>
                <For each={page().containers}>{container => <button classList={{selected:group()==='account:'+container.id}} onClick={() => goList('account:'+container.id)}><span>{text(container.data.name)}</span></button>}</For><For each={page().groups}>{entry => <button classList={{selected:group()===entry.group.id}} onClick={() => goList(entry.group.id)}><span>{text(entry.group.data.name)}</span><span>{entry.count}</span></button>}</For>
            </nav><footer><AppMenu/></footer>
        </aside>
        <section class="contacts-list-pane" aria-label={s.app} aria-hidden={compact() && pane() !== 'contacts'}>
            <header class="contacts-list-header"><Show when={compact()}><button class="contact-back" onClick={back}>{s.backLists}</button></Show><h1>{title()}</h1><Show when={canWrite('contacts.create')}><button class="contact-new" aria-label="New Contact" disabled={commands.pending()||commands.uncertain().length>0} onClick={create}>New Contact</button></Show>
                <div class="contact-selection-toolbar"><button class="contact-back" onClick={()=>{setSelecting(value=>!value);setChecked(new Set<string>());}}>{selecting()?'Done':'Select'}</button><Show when={selecting()}><span role="status">{selectedContacts().length} selected (up to 32)</span><button class="contact-back" disabled={!selectedContacts().length} onClick={()=>downloadContacts(selectedContacts())}>Export selected</button><Show when={canWrite('contacts.delete')}><button class="contact-back" disabled={!selectedContacts().length||commands.pending()||selectedContacts().some(item=>!canWrite('contacts.delete',item))} onClick={()=>setDeleting(selectedContacts())}>Delete selected</button></Show></Show></div>
                <input ref={searchField} type="search" aria-label={s.search} placeholder={s.search} value={search()} onInput={event => setSearch(event.currentTarget.value)}/>
                <div class="contact-preferences"><label>{s.sort}<select aria-label={s.sort} value={preferences.sort()} onChange={event => preferences.sortBy(event.currentTarget.value)}><option value="given">{s.given}</option><option value="family">{s.family}</option></select></label><label>{s.display}<select aria-label={s.display} value={preferences.display()} onChange={event => preferences.displayAs(event.currentTarget.value)}><option value="given">{s.given}</option><option value="family">{s.family}</option></select></label></div>
            </header>
            <Show when={me() && !search().trim()}><button class="contact-my-card" aria-pressed={selecting()?checked().has(me()!.contact.id):undefined} onClick={event => open(me()!,event.currentTarget.querySelector('.avatar'))}><ContactAvatar item={me()!}/><span><strong>{me()!.display_name}</strong><small>{s.myCard}</small></span></button></Show>
            <Show when={!page().error} fallback={<div class="empty" role="alert"><p>{s.unavailable}</p><button class="button subtle" onClick={() => setRetry(value=>value+1)}>{s.retry}</button></div>}>
                <Show when={page().sections} fallback={<p class="contact-loading" role="status">{auth.live()?.status() === 'offline' ? s.unavailable : s.loading}</p>}>
                    <Show when={rows().length} fallback={<p class="contact-loading" role="status">{search().trim() ? s.noResults : me() ? '' : s.empty}</p>}>
                        <div class="contacts-indexed-list" onKeyDown={event => { if (event.key === 'Enter' && event.target === event.currentTarget.querySelector('.virtual-viewport')) { const item = rows().find(row => row.item.contact.id === active())?.item ?? rows()[0]?.item; if (item) { event.preventDefault(); open(item); } } }}>
                            <VirtualList items={rows()} keyFor={row=>row.item.contact.id} estimate={row=>row.first?88:60} fixedHeights ref={value=>list=value} label={s.app} selectedKey={active() || id()} onActive={row=>setActive(row.item.contact.id)} windowKey={group()+preferences.sort()+search()} render={row=><div class="contact-row-group"><Show when={row.first}><h2 class="contact-section">{row.section}</h2></Show><button class="contact-row" data-contact={row.item.contact.id} aria-label={row.item.display_name} aria-pressed={selecting()?checked().has(row.item.contact.id):undefined} classList={{'contact-selected':selecting()&&checked().has(row.item.contact.id)}} onClick={event=>open(row.item,event.currentTarget.querySelector('.avatar'))}><ContactAvatar item={row.item}/><span><strong>{row.item.display_name}</strong><small>{text(row.item.contact.data.organization)}</small></span></button></div>}/>
                            <nav class="contact-index" aria-label={s.index} onPointerDown={event=>{event.preventDefault();event.currentTarget.setPointerCapture(event.pointerId);scrub(event);}} onPointerMove={event=>{if(event.currentTarget.hasPointerCapture(event.pointerId))scrub(event);}} onPointerUp={()=>setBubble('')} onPointerCancel={()=>setBubble('')}><For each={letters}>{letter=><button tabindex="0" aria-label={'Jump to '+letter} onClick={()=>{jump(letter);setBubble('');}}>{letter}</button>}</For></nav><Show when={bubble()}><span class="contact-letter-bubble" aria-hidden="true">{bubble()}</span></Show>
                        </div>
                    </Show>
                </Show>
            </Show>
        </section>
        <section ref={card} class="contacts-card-pane" aria-label={s.contact} aria-hidden={compact() && pane() !== 'card'}>
            <Show when={commands.error()}><div class="contact-write-status" role="alert"><p>{commands.error()}</p><Show when={lastCommand() && (!lastBefore || selected())}><button onClick={review}>Review edit</button></Show></div></Show>
            <Show when={commands.pending()}><p class="contact-write-status" role="status">Saving…</p></Show>
            <For each={commands.uncertain()}>{command=><div class="contact-write-status" role="status"><p>Not sure if saved. Check before trying again.</p><button onClick={()=>commands.check(command)}>Check status</button></div>}</For>
            <Show when={editing() || isNew()?editorRevision():0} keyed fallback={<><div class="contact-write-toolbar"><Show when={id()}><button class="contact-back" onClick={back}>{s.back}</button></Show><Show when={selected() && canWrite('contacts.update',selected())}><button class="contact-back" onClick={()=>edit(selected())}>Edit</button></Show></div><Show when={selected()} fallback={<div class="empty"><p>{id() ? page().sections ? s.unavailable : s.loading : s.select}</p></div>}>{item=><Card item={item()}/>}</Show></>}>
                {_revision=> <ContactEditor item={editorItem()} initial={recovery()} busy={commands.pending()} cancel={()=>{setEditing(false);if(isNew())back();}} save={save} remove={editorItem() && canWrite('contacts.delete',editorItem())?()=>setDeleting([editorItem()!]):undefined}/>}
            </Show>
        </section>
        <Show when={deleting()}>{item=><dialog class="contact-delete-dialog" aria-label="Delete Contact" ref={element=>queueMicrotask(()=>element.showModal())} onClose={()=>setDeleting(undefined)}><h2>Delete Contact?</h2><p>{item().length===1?item()[0].display_name:`Delete ${item().length} contacts?`}</p><footer><button class="button subtle" autofocus onClick={()=>setDeleting(undefined)}>Cancel</button><button class="button danger" aria-label="Confirm Delete Contact" onClick={event=>{for(const contact of item()){const command=commands.issue(event,'contacts.delete',{id:contact.contact.id},contact);if(command){lastBefore=contact;lastArgs={};setLastCommand(command);}}setDeleting(undefined);setSelecting(false);setChecked(new Set<string>());back();}}>Delete</button></footer></dialog>}</Show>
    </main>;
}
