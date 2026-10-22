import { For, Show, onCleanup } from 'solid-js';
import type { BridgeRecord } from '../data/types';
import { reducedMotion, springAnimation } from '../motion/animate';
import { Icon } from '../design/Icon';
import { mediaSource } from '../messages/media/source';
import { civil, dateLabel, day, safeURL, text } from './store';
export function Inspector(props: { record?: BridgeRecord; calendar?: BridgeRecord; close: () => void; writable?: boolean; edit?: (record: BridgeRecord) => void; remove?: (record: BridgeRecord) => void }) {
    let animation: ReturnType<typeof springAnimation> | undefined;
    onCleanup(() => animation?.cancel());
    const show = (element: HTMLDialogElement) => queueMicrotask(() => {
        element.showModal();
        if (innerWidth < 1024 && !reducedMotion()) animation = springAnimation(value => { element.style.transform = `translateX(${value}%)`; }, 100, 0, { response: .35, damping: 1 }, 0, () => { element.style.transform = ''; });
    });
    const data = () => props.record?.data ?? {};
    const date = (field: string) => {
        const value = text(data()[field]), parsed = data().all_day ? civil(value) : new Date(value);
        if (!Number.isFinite(parsed.getTime())) return value;
        if (data().all_day) return dateLabel(field === 'end' ? day(parsed, -1) : parsed, { dateStyle: 'long' });
        try { return dateLabel(parsed, { dateStyle: 'long', timeStyle: 'short', timeZone: text(data().time_zone) || undefined }); }
        catch { return dateLabel(parsed, { dateStyle: 'long', timeStyle: 'short' }); }
    };
    const repeat = () => (Array.isArray(data().recurrence) ? data().recurrence as Record<string, unknown>[] : []).map(rule => `Every ${Number(rule.interval) > 1 ? Number(rule.interval) + ' ' : ''}${text(rule.frequency)}${Number(rule.count) > 0 ? ', ' + Number(rule.count) + ' times' : text(rule.end) ? ', until ' + text(rule.end) : ''}`).join('; ');
    const alerts = () => (Array.isArray(data().alarms) ? data().alarms as Record<string, unknown>[] : []).map(alarm => typeof alarm.relative_offset === 'number' ? alarm.relative_offset === 0 ? 'At time of event' : `${Math.abs(alarm.relative_offset) / 60} minutes ${alarm.relative_offset < 0 ? 'before' : 'after'}` : text(alarm.absolute_date)).join('; ');
    const attendees = () => Array.isArray(data().attendees) ? data().attendees as Record<string, unknown>[] : [];
    const coverage = (field: string) => {
        const fields = data().store_coverage as Record<string, { status?: string }> | undefined;
        return fields?.[field]?.status;
    };
    const attachments = () => {
        if (!['complete', 'partial'].includes(coverage('attachments') ?? '')) return [];
        const items = Array.isArray(data().attachments) ? data().attachments as Record<string, unknown>[] : [];
        return items.filter(attachment => {
            const blob = attachment.blob as Record<string, unknown> | undefined;
            return blob?.state === 'present' && props.record && mediaSource({ ...props.record, data: attachment }, 'original');
        });
    };
    const noTravel = () => coverage('travel_time') === 'complete' && data().travel_time === 0;
    const status = (value: unknown) => ({ 0: 'Unknown', 1: 'Pending', 2: 'Accepted', 3: 'Declined', 4: 'Tentative', 5: 'Delegated', 6: 'Completed', 7: 'In progress' })[Number(value)] ?? 'Unknown';
    return <dialog class="calendar-inspector" aria-label="Event details" ref={show} onClose={props.close} onClick={event => { if (event.target === event.currentTarget) { const rect = event.currentTarget.getBoundingClientRect(); if (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) props.close(); } }}><header><h2>Event details</h2><button class="icon-button" autofocus aria-label="Close event details" onClick={props.close}><Icon name="close"/></button></header><Show when={props.record} fallback={<p>Event unavailable</p>}><Show when={props.writable && props.record}><div class="calendar-inspector-actions"><button onClick={() => props.edit?.(props.record!)}>Edit event</button><button class="destructive" onClick={() => props.remove?.(props.record!)}>Delete event</button></div></Show><h3>{text(data().title) || 'Untitled event'}</h3><dl><dt>Calendar</dt><dd>{text(props.calendar?.data.title)}</dd><dt>Starts</dt><dd>{date('start')}<Show when={data().all_day}> · All-day</Show></dd><dt>Ends</dt><dd>{date('end')}</dd><Show when={!data().all_day && data().time_zone}><dt>Time zone</dt><dd>{text(data().time_zone)}</dd></Show><Show when={repeat()}><dt>Repeat</dt><dd>{repeat()}</dd></Show><Show when={alerts()}><dt>Alert</dt><dd>{alerts()}</dd></Show><Show when={data().availability !== undefined && data().availability !== null}><dt>Show as</dt><dd>{({ 0: 'Busy', 1: 'Free', 2: 'Tentative', 3: 'Unavailable' })[Number(data().availability)] ?? 'Unknown'}</dd></Show><Show when={noTravel()}><dt>Travel time</dt><dd>None</dd></Show></dl><Show when={data().location}><section><h4>Location</h4><p>{text(data().location)}</p><a href={'https://maps.apple.com/?q=' + encodeURIComponent(text(data().location))} target="_blank" rel="noopener noreferrer">Get directions</a></section></Show><Show when={safeURL(data().conference_url)}>{url => <a class="button" href={url()} target="_blank" rel="noopener noreferrer">Join meeting</a>}</Show><Show when={attendees().length}><section><h4>Invitees</h4><For each={attendees()}>{attendee => <p>{text(attendee.name) || text(attendee.url)}<span class="calendar-secondary"> · {status(attendee.status)}</span></p>}</For></section></Show><Show when={safeURL(data().url)}>{url => <section><h4>URL</h4><a href={url()} target="_blank" rel="noopener noreferrer">{url()}</a></section>}</Show><Show when={data().notes}><section><h4>Notes</h4><p class="calendar-notes">{text(data().notes)}</p></section></Show><Show when={attachments().length}><section><h4>Attachments</h4><For each={attachments()}>{attachment => <Show when={props.record && mediaSource({ ...props.record, data: attachment }, 'original')}>{url => <a class="calendar-attachment" href={url()} target="_blank" rel="noopener noreferrer" download={text(attachment.filename)}>{text(attachment.filename) || 'Attachment'}</a>}</Show>}</For></section></Show></Show></dialog>;

}
