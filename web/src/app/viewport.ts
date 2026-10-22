import {createSignal} from 'solid-js';
const [height,setHeight]=createSignal(window.innerHeight);
export const viewportHeight=height;
const resize=()=>setHeight(window.visualViewport?.height??window.innerHeight);
(window.visualViewport??window).addEventListener('resize',resize);
requestAnimationFrame(resize);
