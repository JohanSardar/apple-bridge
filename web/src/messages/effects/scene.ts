import type {ScreenEffect} from './data';
export const effectDuration=2400;
export type SceneOptions={effect:ScreenEffect;width:number;height:number;text?:string;origin?:{left:number;top:number;width:number;height:number};seed:number};
type Context=CanvasRenderingContext2D|OffscreenCanvasRenderingContext2D;
const colors=['#ff6eae','#ffd25c','#6cd9fc','#a59bff','#5ce5b4','#ff906e'];
export class EffectScene {
 private particles:number[][]=[];
 private echoText?:string;
 constructor(private context:Context,private options:SceneOptions){const points=Array.from(options.text??'');this.echoText=points.length>60?points.slice(0,59).join('')+'…':options.text;let seed=options.seed||1;const random=()=>{seed^=seed<<13;seed^=seed>>>17;seed^=seed<<5;return (seed>>>0)/4294967296;};this.particles=Array.from({length:84},()=>Array.from({length:6},random));}
 private heart(x:number,y:number,size:number,color:string){const c=this.context;c.save();c.translate(x,y);c.scale(size,size);c.beginPath();c.moveTo(0,.35);c.bezierCurveTo(-1,-.2,-.7,-1,.0,-.5);c.bezierCurveTo(.7,-1,1,-.2,0,.35);c.fillStyle=color;c.fill();c.restore();}
 private capsule(x:number,y:number,w:number,h:number){const c=this.context,r=Math.min(18,h/2,w/2);c.beginPath();c.moveTo(x+r,y);c.arcTo(x+w,y,x+w,y+h,r);c.arcTo(x+w,y+h,x,y+h,r);c.arcTo(x,y+h,x,y,r);c.arcTo(x,y,x+w,y,r);c.closePath();}
 draw(milliseconds:number):boolean {
  const c=this.context,{width:w,height:h,effect,origin}=this.options,t=Math.max(0,milliseconds)/1000,progress=t/(effectDuration/1000),fade=Math.max(0,Math.min(1,t/.12,(2.4-t)/.35));c.clearRect(0,0,w,h);if(progress>=1)return false;c.globalAlpha=fade;c.globalCompositeOperation='source-over';
  const x=origin?origin.left+origin.width/2:w/2,y=origin?origin.top+origin.height/2:h*.44;
  if(effect==='spotlight'){
   const radius=Math.max(w,h)*.85,gradient=c.createRadialGradient(x,y,42+Math.sin(t*2)*4,x,y,radius);gradient.addColorStop(0,'rgba(12,16,27,0)');gradient.addColorStop(.4,'rgba(12,16,27,.4)');gradient.addColorStop(1,'rgba(5,7,16,.78)');c.fillStyle=gradient;c.fillRect(0,0,w,h);c.fillStyle='rgba(255,234,192,.09)';c.beginPath();c.moveTo(w*.48,-30);c.lineTo(x-85,y+90);c.quadraticCurveTo(x,y+160,x+85,y+90);c.closePath();c.fill();
  }else if(effect==='echo'){
   for(let i=0;i<14;i++){const p=(t-i*.1)/1.1;if(p<0||p>1)continue;const scale=1+p*2.5,bw=(origin?.width??180)*scale,bh=(origin?.height??62)*scale;c.save();c.translate(x,y);c.rotate(Math.sin(i*.8)*p*.14);c.globalAlpha=fade*(1-p)*.48;c.fillStyle=colors[i%colors.length];this.capsule(-bw/2,-bh/2,bw,bh);c.fill();if(this.echoText){c.fillStyle='#ffffff';c.font=`600 ${15*scale}px system-ui`;c.textAlign='center';c.textBaseline='middle';c.fillText(this.echoText,0,0,Math.max(1,bw-28*scale));}c.restore();}
  }else if(effect==='balloons'){
   for(let i=0;i<12;i++){const p=this.particles[i],rise=Math.min(1,progress*(.85+p[3]*.3)),bx=w*(.05+p[0]*.9)+Math.sin(t*2+p[2]*8)*16,by=h+100-(h+260)*rise,size=24+p[1]*16;c.save();c.translate(bx,by);c.rotate(Math.sin(t+p[2]*8)*.09);c.strokeStyle='rgba(175,175,191,.65)';c.lineWidth=1;c.beginPath();c.moveTo(0,size);c.bezierCurveTo(-12,size+28,12,size+50,-5,size+94);c.stroke();c.fillStyle=colors[i%colors.length];c.beginPath();c.ellipse(0,0,size*.78,size,0,0,Math.PI*2);c.fill();c.fillStyle='rgba(255,255,255,.4)';c.beginPath();c.ellipse(-size*.27,-size*.36,size*.12,size*.27,.4,0,Math.PI*2);c.fill();c.beginPath();c.moveTo(0,size-1);c.lineTo(-4,size+6);c.lineTo(4,size+6);c.fill();c.restore();}
  }else if(effect==='love'){
   for(let i=0;i<28;i++){const p=this.particles[i],phase=Math.min(1,progress*(.8+p[3]*.4)),spread=30+phase*w*.6;c.globalAlpha=fade*(.3+p[4]*.7);this.heart(x+(p[0]-.5)*spread*2+Math.sin(t*2+p[1]*6)*12,y+70-phase*(h*.8)-(p[2]-.5)*70,10+p[1]*30,colors[i%2===0?0:5]);}
  }else if(effect==='lasers'){
   c.globalCompositeOperation='lighter';for(let i=0;i<12;i++){const p=this.particles[i],sx=i%2===0?-15:w+15,sy=h+15,ex=w*(p[0]*1.4-.2)+Math.sin(t*.9+i)*w*.2,ey=-80;c.globalAlpha=fade*(.35+p[2]*.45);c.strokeStyle=colors[i%colors.length];c.shadowColor=c.strokeStyle;c.shadowBlur=8;c.lineWidth=1+p[1]*2;c.beginPath();c.moveTo(sx,sy);c.lineTo(ex,ey);c.stroke();}c.shadowBlur=0;
  }else if(effect==='fireworks'){
   this.fireworks(t,fade,5);
  }else{
   for(let i=0;i<84;i++){const p=this.particles[i],flight=progress*(.7+p[3]*.45),cx=p[0]*w+Math.sin(t*(1+p[2])+p[5]*9)*35,cy=-70+flight*(h+170)+p[1]*100;c.save();c.translate(cx,cy);c.rotate(p[4]*Math.PI*2+t*(1+p[1]*4));c.globalAlpha=fade*(.55+p[5]*.45);c.fillStyle=effect==='celebration'?['#ffd770','#fff0bc','#d4adff','#ffffff'][i%4]:colors[i%colors.length];const pw=3+p[1]*5,ph=5+p[2]*9;if(i%5===0){c.beginPath();c.ellipse(0,0,pw,pw,0,0,Math.PI*2);c.fill();}else c.fillRect(-pw/2,-ph/2,pw,ph*Math.max(.15,Math.abs(Math.sin(t*4+p[3]*7))));c.restore();}if(effect==='celebration')this.fireworks(t,fade*.45,3);
  }
  c.globalAlpha=1;c.globalCompositeOperation='source-over';return true;
 }
 private fireworks(t:number,fade:number,groups:number){const c=this.context,{width:w,height:h}=this.options;for(let group=0;group<groups;group++){const p=this.particles[group],age=t-group*.25-.08;if(age<0||age>1.1)continue;const bx=w*(.15+p[0]*.7),by=h*(.16+p[1]*.45),speed=130+p[2]*130;c.globalAlpha=fade*(1-age/1.1);for(let i=0;i<16;i++){const angle=i*Math.PI/8+p[3],dx=Math.cos(angle)*speed*age,dy=Math.sin(angle)*speed*age+70*age*age;c.strokeStyle=colors[group%colors.length];c.lineWidth=1.5;c.beginPath();c.moveTo(bx+dx*.83,by+dy*.83);c.lineTo(bx+dx,by+dy);c.stroke();c.fillStyle=colors[group%colors.length];c.beginPath();c.arc(bx+dx,by+dy,2.4,0,Math.PI*2);c.fill();}}}
}
