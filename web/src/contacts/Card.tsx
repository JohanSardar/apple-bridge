import { createMemo, For, Show } from 'solid-js';
import { router } from '../app/router';
import { Avatar } from '../messages/list/Avatar';
import { addressText, dateText, label, labeled, object, safeURL, text, type Item } from './store';
import { downloadContacts } from './vcard';
import { s } from './strings';
export function ContactAvatar(props: { item: Item }) {
    return <Avatar variant={props.item.avatar?.variant==='display'?'display':'thumb'} title={props.item.display_name} handles={new Map()} photo={props.item.avatar ? { digest: props.item.avatar.digest, record: props.item.contact.id } : undefined}/>;
}
export function Card(props: { item: Item }) {
    const data = createMemo(() => props.item.contact.data);
    const phone = () => text(labeled(data().phones)[0]?.value), email = () => text(labeled(data().emails)[0]?.value);
    const recipient = () => phone() || email();
    const phonetic = () => ['phonetic_given_name','phonetic_middle_name','phonetic_family_name','phonetic_organization'].map(key => text(data()[key])).filter(Boolean).join(' ');
    return <article class="contact-card" aria-label={props.item.display_name}>
        <header class="contact-heading"><ContactAvatar item={props.item}/><h1 tabindex="-1">{props.item.display_name}</h1><Show when={props.item.is_me === true}><p>{s.myCard}</p></Show><Show when={phonetic()}><p aria-label={s.phonetic}>{phonetic()}</p></Show><p>{[text(data().job_title), text(data().department), text(data().organization)].filter(Boolean).join(' · ')}</p></header>
        <nav class="contact-actions" aria-label="Quick actions">
            <Show when={recipient()}><button onClick={() => router.navigate('/messages/new?to=' + encodeURIComponent(recipient()))}>{s.message}</button></Show>
            <Show when={phone()}><a href={'tel:' + encodeURIComponent(phone())}>{s.call}</a></Show>
            <Show when={recipient()}><a href={'facetime:' + encodeURIComponent(recipient())}>{s.video}</a></Show>
            <Show when={email()}><a href={'mailto:' + encodeURIComponent(email())}>{s.mail}</a></Show>
        </nav>
        <dl class="contact-fields">
            <For each={[['name_prefix','prefix'],['name_suffix','suffix'],['nickname','nickname'],['previous_family_name','previous last name']] as const}>{([key,name]) => <Show when={text(data()[key])}><div class="contact-field"><dt>{name}</dt><dd>{text(data()[key])}</dd></div></Show>}</For>
            <For each={['phones','emails','urls','postal_addresses','relations','social_profiles','instant_messages'] as const}>{key => <For each={labeled(data()[key])}>{entry => {
                const value = () => key === 'postal_addresses' ? addressText(entry.value) : ['social_profiles','instant_messages'].includes(key) ? [text(object(entry.value).username),text(object(entry.value).service)].filter(Boolean).join(' · ') : text(entry.value);
                const href = () => key === 'phones' ? 'tel:' + encodeURIComponent(value()) : key === 'emails' ? 'mailto:' + encodeURIComponent(value()) : key === 'urls' ? safeURL(value()) : key === 'social_profiles' ? safeURL(object(entry.value).url) : key === 'postal_addresses' ? 'https://maps.apple.com/?q=' + encodeURIComponent(value()) : undefined;
                return <Show when={value()}><div class="contact-field"><dt>{label(entry.label, s[key])}</dt><dd><Show when={href()} fallback={value()}><a href={href()} target={key === 'postal_addresses' || key === 'urls' || key === 'social_profiles' ? '_blank' : undefined} rel="noreferrer">{value()}</a></Show></dd></div></Show>;
            }}</For>}</For>
            <For each={['birthday','non_gregorian_birthday']}>{key => <Show when={dateText(data()[key])}><div class="contact-field"><dt>{key === 'birthday' ? s.birthday : s.otherBirthday}</dt><dd>{dateText(data()[key])}</dd></div></Show>}</For>
            <For each={labeled(data().dates)}>{entry => <div class="contact-field"><dt>{label(entry.label,s.dates)}</dt><dd>{dateText(entry.value)}</dd></div>}</For>
        </dl>
        <Show when={props.item.conversation_ids.length}><section class="contact-conversations"><h2>{s.conversations}</h2><For each={props.item.conversation_ids}>{id => <button class="button subtle" onClick={() => router.navigate('/messages/' + encodeURIComponent(id))}>{s.message}</button>}</For></section></Show>
        <button class="button subtle contact-export" onClick={() => downloadContacts([props.item])}>{s.export}</button>
    </article>;
}
