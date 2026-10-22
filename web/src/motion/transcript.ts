import {crossfade,reducedMotion,springAnimation} from './animate';
export function receiveBubble(element:HTMLElement,sent:boolean){
    element.dataset.motion='receive';
    element.style.transformOrigin=sent?'right bottom':'left bottom';
    const reset=()=>{element.style.transform='';element.style.transformOrigin='';delete element.dataset.motion;};
    if(reducedMotion()){
        const animation=crossfade(element);animation.onfinish=reset;
        return ()=>{animation.cancel();reset();};
    }
    const animation=springAnimation(value=>{element.style.transform=`scale(${value})`;},.92,1,{response:.38,damping:.75},0,reset);
    return ()=>{animation.cancel();reset();};
}
