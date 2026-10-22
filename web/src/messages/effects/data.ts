export const screenEffects=['echo','spotlight','balloons','confetti','love','lasers','fireworks','celebration'] as const;
export const bubbleEffects=['slam','loud','gentle','invisible-ink'] as const;
export const textEffects=['big','small','shake','nod','explode','ripple','bloom','jitter'] as const;
export type ScreenEffect=typeof screenEffects[number];
export type BubbleEffect=typeof bubbleEffects[number];
export type TextEffect=typeof textEffects[number];
export type MessageEffect={scope:'screen';name:ScreenEffect;label:string}|{scope:'bubble';name:BubbleEffect;label:string};
const aliases:Record<string,ScreenEffect|BubbleEffect>={impact:'slam',slam:'slam',loud:'loud',gentle:'gentle',invisibleink:'invisible-ink',echo:'echo',ckechoeffect:'echo',spotlight:'spotlight',ckspotlighteffect:'spotlight',balloons:'balloons',ckhappybirthdayeffect:'balloons',confetti:'confetti',ckconfettieffect:'confetti',love:'love',ckhearteffect:'love',lasers:'lasers',cklaserseffect:'lasers',fireworks:'fireworks',ckfireworkseffect:'fireworks',celebration:'celebration',cksparkleseffect:'celebration'};
export function normalizeEffect(raw:unknown):MessageEffect|undefined {
 if(typeof raw!=='string'||raw.length>256)return;
 const token=raw.trim().split('.').at(-1)?.toLowerCase().replace(/[\s_-]/gu,''),name=token&&Object.hasOwn(aliases,token)?aliases[token]:undefined;if(!name)return;
 const title=name==='invisible-ink'?'Invisible Ink':name[0].toUpperCase()+name.slice(1),label='Sent with '+title;
 return (screenEffects as readonly string[]).includes(name)?{scope:'screen',name:name as ScreenEffect,label}:{scope:'bubble',name:name as BubbleEffect,label};
}
export function effectLabel(raw:unknown):string|undefined {return normalizeEffect(raw)?.label||(typeof raw==='string'&&raw.trim()?'Sent with an effect':undefined);}
