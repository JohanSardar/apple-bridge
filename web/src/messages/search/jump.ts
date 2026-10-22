import { router } from '../../app/router';
import { reducedMotion } from '../../motion/animate';
let cancelPending:(()=>void)|undefined;
export function jumpToMessage(chatID:string,messageID:string){
    cancelPending?.();
    router.navigate('/messages/'+encodeURIComponent(chatID)+'/'+encodeURIComponent(messageID));
    let frame=0,timer=0,animation:Animation|undefined,target:HTMLElement|undefined,done=false;
    const observer=new MutationObserver(()=>{cancelAnimationFrame(frame);frame=requestAnimationFrame(find);});
    const stop=()=>{if(done)return;done=true;observer.disconnect();cancelAnimationFrame(frame);clearTimeout(timer);animation?.cancel();target?.removeAttribute('data-search-hit');if(cancelPending===stop)cancelPending=undefined;};
    function find(){
        if(router.route().chat!==chatID||router.route().message!==messageID){stop();return;}
        const row=document.querySelector<HTMLElement>('[data-message="'+CSS.escape(messageID)+'"]');
        if(!row||row.closest('[aria-busy=true]'))return;
        const box=row.getBoundingClientRect();if(!box.width||!box.height||box.bottom<0||box.top>innerHeight)return;
        observer.disconnect();clearTimeout(timer);target=row;row.setAttribute('data-search-hit','');
        const surface=row.querySelector<HTMLElement>('.bubble')??row;
        const blue=getComputedStyle(document.documentElement).getPropertyValue('--blue').trim();
        animation=reducedMotion()?surface.animate([{opacity:1},{opacity:.75},{opacity:1}],{duration:150}):surface.animate([{filter:'none'},{filter:`drop-shadow(0 0 6px ${blue}) brightness(1.12)`,offset:.3},{filter:'none',offset:.6},{filter:`drop-shadow(0 0 4px ${blue}) brightness(1.08)`,offset:.8},{filter:'none'}],{duration:900,easing:'ease-out'});
        void animation.finished.then(stop).catch(stop);
    }
    cancelPending=stop;observer.observe(document,{childList:true,subtree:true,attributes:true,attributeFilter:['aria-busy','style']});timer=window.setTimeout(stop,10000);frame=requestAnimationFrame(find);
}
