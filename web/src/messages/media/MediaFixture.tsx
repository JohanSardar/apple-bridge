import { For, onCleanup } from 'solid-js';
import { auth } from '../../auth/session';
import { fields, text } from '../store/types';
import { MessageMedia } from './MessageMedia';
export function MediaFixture() {
    const view = auth.live()?.subscribe('thread', { chat_id: new URLSearchParams(location.search).get('chat'), limit: 200 });
    onCleanup(() => view?.dispose());
    const items = () => (view?.data().items ?? []).filter(item => !item.data.balloon).flatMap(message => message.attachments.filter(attachment => text(attachment.data.mime_type).startsWith('image/') && !attachment.data.hidden && !attachment.data.genmoji && !attachment.data.is_sticker).map(attachment => ({ message, attachment })));
    return <section class="media-fixture" aria-label="Media bubbles"><For each={items()}>{value => {
        const dimensions=fields(value.attachment.data.media),width=Number(dimensions.width)||260,height=Number(dimensions.height)||120,scale=Math.min(1,(matchMedia('(max-width:699px)').matches?240:320)/width,360/height);
        return <div class="media-fixture-slot" data-message={value.message.id} style={{width:`${width*scale}px`,height:`${height*scale}px`}}><MessageMedia message={value.message} part={{index:0,kind:'attachment',attachment_id:value.attachment.id}} width={width*scale} height={height*scale}/></div>;
    }}</For></section>;
}
