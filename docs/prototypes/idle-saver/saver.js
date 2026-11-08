// DEs-UI boot wordmark mask, combined from compositor/assets/wordmark.la.
const WORDMARK = "data:image/png;base64,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";

const get = id => document.getElementById(id);
const canvas = get("waves");
const ctx = canvas.getContext("2d", { alpha: false });
const studio = get("studio"), screen = get("screen");
const slider = get("phase"), phaseLabel = get("phase-label");
const reduce = matchMedia("(prefers-reduced-motion: reduce)");
const scenes=globalThis.desSaverScenes;
const presets = {
  silk: {name:"Silk Fold", description:"Magenta dots travel across folds of silk and gather into the DEs-UI wordmark.", ink:"#ff00f1", highlight:"#ff9ce0", background:"#050208"},
  "silk-ink": {name:"Silk Fold · Ink", description:"A deep magenta DEs-UI wordmark stays woven into the moving field of silk dots.", ink:"#ad227b", highlight:"#d765ac", background:"#050208"},
  contour: {name:"Signal Contour", description:"Fine pink contour lines bend and become slices of the DEs-UI wordmark.", ink:"#ed56be", highlight:"#ffd1ec", background:"#06040a"},
  terminal: {name:"Terminal Weave", description:"Magenta and pale cyan glyphs drift across a grid and assemble the DEs-UI wordmark.", ink:"#d44ab6", highlight:"#9ef5e1", background:"#040609"},
  ...scenes
};
const sceneLevels=new Map(Object.entries(scenes).map(([key,scene])=>[key,scene.level]));
const clamp = (n,a=0,b=1) => Math.min(b,Math.max(a,n));
const mix = (a,b,t) => a + (b-a)*t;
const smooth = (a,b,t) => {const p=clamp((t-a)/(b-a));return p*p*(3-2*p)};
const noise = (x,y) => {const v=Math.sin(x*127.1+y*311.7)*43758.5453;return v-Math.floor(v)};
const tau = Math.PI*2;
const glyphs = ".:+*#%@";
let w=1,h=1,dpr=1,mode="silk",phase=reduce.matches?1:0,cycle=0.4;
let paused=reduce.matches, inView=true, frame=0, last=0, elapsed=0;
let motionOverride=false;
const motionAllowed=()=>!reduce.matches||motionOverride;
let logoDots=[],logoRuns=[];
let logoMask, brandMask;
const brandTints=new Map();
const pointer={x:0,y:0,energy:0,inside:false};

function logoAt(step) {
  const image=document.createElement("canvas");
  image.width=642; image.height=126;
  const m=image.getContext("2d",{willReadFrequently:true});
  const logo=new Image();
  logo.onload=()=>{
    m.drawImage(logo,0,0);
    const samples=m.getImageData(0,0,642,126).data;
    logoMask=samples;
    const mask=m.createImageData(642,126);
    for(let i=0;i<samples.length;i+=4){
      mask.data[i]=mask.data[i+1]=mask.data[i+2]=255;
      mask.data[i+3]=samples[i];
    }
    m.putImageData(mask,0,0);brandMask=image;
    const filled=(x,y)=>samples[(y*642+x)*4]>115;
    for(let y=2;y<126;y+=step){
      for(let x=2;x<642;x+=step) {
        if(filled(x,y))logoDots.push({x:x/642,y:y/126,seed:noise(x,y),fuzz:noise(y,x)});
      }
    }
    for(let y=4;y<126;y+=5) {
      let left=-1;
      for(let x=0;x<=642;x++) {
        if(x<642&&filled(x,y)) {if(left<0)left=x;}
        else if(left>=0) {
          if(x-left>2) logoRuns.push({a:left/642,b:x/642,y:y/126,seed:noise(left,y)});
          left=-1;
        }
      }
    }
    invalidate();
  };
  logo.src=WORDMARK;
}

function drawBrand(x,y,width,color,opacity){
  if(!brandMask)return;
  if(!brandTints.has(color)){
    const tinted=document.createElement("canvas");
    tinted.width=brandMask.width;tinted.height=brandMask.height;
    const paint=tinted.getContext("2d");
    paint.drawImage(brandMask,0,0);paint.globalCompositeOperation="source-in";
    paint.fillStyle=color;paint.fillRect(0,0,tinted.width,tinted.height);
    brandTints.set(color,tinted);
  }
  const height=width*126/642;
  ctx.save();ctx.globalAlpha=opacity;
  ctx.drawImage(brandTints.get(color),x-width/2,y-height/2,width,height);ctx.restore();
}

function surface(x,y,t) {
  const u=x/w, v=y/h;
  const folds= Math.sin(u*13.5+v*5.2+1.18*Math.sin(v*5.2-u*3.5)-t*.63);
  const under=Math.sin(u*24.0-v*3.6+t*.5);
  return folds*.77+under*.23;
}
function ripple(x,y) {
  if(pointer.energy<.01)return 0;
  const d=Math.hypot(x-pointer.x,y-pointer.y);
  return Math.sin(d*.095-elapsed*5.6)*Math.exp(-d/64)*pointer.energy;
}
function background(p) {
  ctx.globalAlpha=1;
  ctx.fillStyle=p.background;ctx.fillRect(0,0,w,h);
  const halo=ctx.createRadialGradient(w*.51,h*.51,0,w*.51,h*.51,Math.max(w*.45,h*.65));
  halo.addColorStop(0,mode==="contour"?"#1f0b1f":mode==="terminal"?"#10101c":"#220719");
  halo.addColorStop(.62,mode==="terminal"?"#080a12":"#100614");
  halo.addColorStop(1,p.background);
  ctx.fillStyle=halo;ctx.fillRect(0,0,w,h);
}
function field(p,k) {
  const woven=mode==="silk-ink";
  const step=mode==="terminal"?Math.max(10,w/116):woven?Math.max(4.6,w/205):Math.max(6.7,w/174);
  const lw=Math.min(w*.75,h*1.32,690), lh=lw*126/642;
  if(mode==="terminal")ctx.font=`${Math.max(7,step*.85)}px JetBrains, monospace`;
  for(let y=-step;y<h+step;y+=step)for(let x=0;x<w;x+=step) {
    const height=surface(x,y,elapsed), slope=surface(x+step,y,elapsed)-height;
    const edge=1-smooth(.62,1.12,Math.hypot((x/w-.5)*1.7,(y/h-.51)*1.75));
    const band=1-smooth(.1,.60,Math.abs(y/h-.51+height*.2));
    const mid=1-smooth(.08,.65,Math.abs(y/h-.5)*1.7+Math.abs(x/w-.5)*.70);
    const light=clamp(.32 + slope*2.6 + height*.3);
    const r=ripple(x,y);
    const py=y+height*h*.103+r*16;
    // The logo changes the dye and weight of the very same moving dots.
    const u=(x-(w-lw)*.5)/lw, v=(py-r*5-height*h*.018-(h-lh)*.5)/lh;
    const mark=woven&&logoMask&&u>=0&&u<1&&v>=0&&v<1?logoMask[((v*126|0)*642+(u*642|0))*4]/255:0;
    const dye=smooth(.23,.75,mark);
    const foldOpacity=edge*(.12+band*.88)*(.19+light*.68);
    const opacity=woven?mix(foldOpacity*(.66-mid*.36),.68+.24*light+.08*k,dye):foldOpacity*(1-k*(.43+mid*.52));
    if(opacity<.024)continue;
    ctx.globalAlpha=opacity;
    if(mode==="terminal") {
      const seed=noise(x/step|0,y/step|0);
      const ch=glyphs[Math.min(glyphs.length-1,Math.floor(clamp(light*.69+seed*.3)*glyphs.length))];
      ctx.fillStyle=seed>.92?p.highlight:p.ink;
      ctx.fillText(ch,x,py);
    } else {
      const radius=clamp(step*(.115+light*.25+dye*(.16+.045*k)),.6,3.8);
      ctx.fillStyle=dye>.3?(light>.54?"#e054a7":"#b61d70"):light>.83?p.highlight:p.ink;
      ctx.beginPath();ctx.arc(x,py,radius,0,tau);ctx.fill();
    }
  }
}
function contours(p,k) {
  const step=Math.max(10,h/42);
  for(let row=0,y=-h*.1;y<h*1.10;y+=step,row++){
    const local=y/h-.53;
    ctx.lineWidth=row%6===0?1.2:.65;
    ctx.strokeStyle=row%6===0?p.highlight:p.ink;
    ctx.beginPath();
    for(let x=-8;x<w+9;x+=8){
      const crest=surface(x,y,elapsed);
      const v=y+crest*h*.14+ripple(x,y)*17;
      if(x===-8)ctx.moveTo(x,v);else ctx.lineTo(x,v);
    }
    ctx.globalAlpha=(1-smooth(.30,.66,Math.abs(local)))*(.33+.16*Math.cos(row*.92+elapsed*.35))*(1-k*(.45+.43*(1-smooth(.04,.45,Math.abs(local)))));
    ctx.stroke();
  }
  // Fade the contour ends into the same dark stage.
  ctx.globalAlpha=1;
  const veil=ctx.createLinearGradient(0,0,w,0);
  veil.addColorStop(0,p.background);veil.addColorStop(.22,"transparent");veil.addColorStop(.80,"transparent");veil.addColorStop(1,p.background);
  ctx.fillStyle=veil;ctx.fillRect(0,0,w,h);
}
function position(u,v,seed,fuzz,k,x0,y0,lw,lh) {
  const tx=x0+u*lw, ty=y0+v*lh;
  const fx=tx+(u-.5)*w*.35+(seed-.5)*22;
  const fy=h*.51+surface(fx,h*.51+(v-.5)*h*.32,elapsed)*h*.15+(v-.5)*h*.32+(fuzz-.5)*11;
  return [mix(fx,tx,k),mix(fy,ty,k)+ripple(tx,ty)*5*(1-k*.72)];
}
function logo(p,k){
  if(k<.025)return;
  const lw=Math.min(w*.72,h*1.28,670),lh=lw*(126/642), x0=(w-lw)*.5,y0=(h-lh)*.5;
  const glow=ctx.createRadialGradient(w*.5,h*.50,0,w*.5,h*.50,lw*.58);
  glow.addColorStop(0,mode==="terminal"?"#171a25":"#341028");glow.addColorStop(1,"transparent");
  ctx.globalAlpha=k*.52;ctx.fillStyle=glow;ctx.fillRect(0,0,w,h);
  if(mode==="contour") {
    ctx.lineCap="round";
    for(const run of logoRuns) {
      const [a,ya]=position(run.a,run.y,run.seed,.48,k,x0,y0,lw,lh);
      const [b,yb]=position(run.b,run.y,run.seed,.48,k,x0,y0,lw,lh);
      ctx.strokeStyle=run.seed>.67?p.highlight:p.ink;
      ctx.globalAlpha=(.27+run.seed*.69)*smooth(.03,.85,k);
      ctx.lineWidth=Math.max(.7,1.8*lw/600);
      ctx.beginPath();ctx.moveTo(a,ya);ctx.lineTo(b,yb);ctx.stroke();
    }
    ctx.lineCap="butt";
    return;
  }
  const spacing=lw*5/642;
  if(mode==="terminal")ctx.font=`${Math.max(4.5,spacing*1.22)}px JetBrains, monospace`;
  for(const q of logoDots){
    if(mode==="terminal"&&q.seed<.18)continue;
    const [x,y]=position(q.x,q.y,q.seed,q.fuzz,k,x0,y0,lw,lh);
    ctx.fillStyle=q.seed>.48?p.highlight:p.ink;
    ctx.globalAlpha=smooth(.03,.9,k)*(.32+.68*q.seed);
    if(mode==="terminal") {
      ctx.fillText(glyphs[Math.floor(q.fuzz*glyphs.length)],x-spacing*.4,y+spacing*.46);
    } else {
      const r=Math.max(.47,spacing*(.29+q.seed*.16));
      ctx.beginPath();ctx.arc(x,y,r,0,tau);ctx.fill();
    }
  }
}
function draw(){
  const p=presets[mode], k=smooth(0,1,phase);
  if(p.draw){
    ctx.save();
    p.draw({ctx,w,h,t:elapsed,p:sceneLevels.get(mode),px:pointer.x/w,py:pointer.y/h,energy:pointer.energy,inside:pointer.inside,motion:motionAllowed(),brand:drawBrand,brandPoints:logoDots});
    ctx.restore();return;
  }
  background(p);
  if(mode==="contour")contours(p,k);else field(p,k);
  if(mode!=="silk-ink")logo(p,k);
  ctx.globalAlpha=1;
}
function updateControls(){
  const scene=scenes[mode];
  get("pause").setAttribute("aria-pressed",String(paused));
  get("pause-label").textContent=paused?"Play":"Pause";
  get("replay").disabled=!motionAllowed();
  slider.value=Math.round((scene?sceneLevels.get(mode):phase)*100);
  get("phase-caption").textContent=scene?.control??(mode==="silk-ink"?"Ink":"Morph");
  phaseLabel.value=scene?slider.value+"%":mode==="silk-ink"?(phase>.5?"Deep":"Soft"):(phase>.94?"Logo":phase>.06?"Forming":"Wave");
  get("motion-note").textContent=!motionAllowed()?"Reduced motion · adjust the slider to explore":scene?.hint??"Move your pointer to ripple the preview";
}
function tick(now){
  frame=0;
  if(!inView||document.hidden)return;
  const dt=last?Math.min((now-last)/1000,.066):.033;
  last=now;
  if(!paused&&motionAllowed()){
    elapsed+=dt;
    if(!scenes[mode]){
      cycle=(cycle+dt)%13;
      phase=smooth(2,5.0,cycle)*(1-smooth(9.3,12.5,cycle));
    }
  }
  pointer.energy=Math.max(0,pointer.energy-dt*2.1);
  draw();updateControls();
  if((!paused&&motionAllowed())||pointer.energy>.02)invalidate();
}
let wake=0;
function invalidate(){
  if(frame||wake||!inView||document.hidden)return;
  // Cap animation draws without making scrubbing or keyboard interaction wait for a long timer.
  wake=setTimeout(()=>{wake=0;frame=requestAnimationFrame(tick);},17);
}
function resize(){
  w=Math.max(1,screen.clientWidth);h=Math.max(1,screen.clientHeight);
  dpr=Math.min(devicePixelRatio||1,1.5,Math.sqrt(1800000/(w*h)));
  canvas.width=Math.ceil(w*dpr);canvas.height=Math.ceil(h*dpr);
  ctx.setTransform(dpr,0,0,dpr,0,0);
  last=0;
  invalidate();
}
new ResizeObserver(resize).observe(screen);
new IntersectionObserver(([entry])=>{inView=entry.isIntersecting;last=0;if(inView)invalidate()}).observe(screen);
document.addEventListener("visibilitychange",()=>{last=0;invalidate()});
reduce.addEventListener("change",()=>{motionOverride=false;paused=reduce.matches;if(paused)phase=1;invalidate()});
for(const type of ["pointermove","pointerdown"])screen.addEventListener(type,e=>{
  if(!motionAllowed())return;
  const rect=screen.getBoundingClientRect();
  pointer.x=e.clientX-rect.left;pointer.y=e.clientY-rect.top;
  pointer.inside=true;
  pointer.energy=type==="pointerdown"?1:.65;
  if(type==="pointerdown"&&e.button===0&&!paused)scenes[mode]?.click?.(clamp(pointer.x/w),elapsed);
  invalidate();
});
screen.addEventListener("pointerleave",()=>{pointer.inside=false;invalidate()});
get("pause").addEventListener("click",()=>{
  paused=!paused;
  if(!paused){motionOverride=reduce.matches;cycle=mix(1.8,5,phase);}
  last=0;invalidate();
});
slider.addEventListener("input",()=>{
  const value=Number(slider.value)/100;
  if(scenes[mode])sceneLevels.set(mode,value);
  else {phase=value;paused=true;}
  invalidate();
});
get("replay").addEventListener("click",()=>{cycle=0;phase=0;elapsed=0;paused=false;last=0;scenes[mode]?.reset?.();invalidate()});
const studies=[...document.querySelectorAll("[data-style]")];
function selectMode(id){
  const button=studies.find(study=>study.dataset.style===id);
  if(!button)return;
  if(mode!==id){mode=id;elapsed=0;last=0;pointer.energy=0;pointer.inside=false;scenes[mode]?.reset?.();}
  for(const item of studies)item.setAttribute("aria-pressed",String(item===button));
  get("preview-name").textContent=presets[mode].name;get("canvas-description").textContent=presets[mode].description;
  updateControls();invalidate();
}
for(const button of studies)button.addEventListener("click",()=>selectMode(button.dataset.style));
function browseBy(delta){
  const i=studies.findIndex(study=>study.dataset.style===mode);
  selectMode(studies[(i+delta+studies.length)%studies.length].dataset.style);
}
get("previous").addEventListener("click",()=>browseBy(-1));
get("next").addEventListener("click",()=>browseBy(1));
get("full").addEventListener("click",async()=>{
  try{
    if(document.fullscreenElement===studio)await document.exitFullscreen();
    else if(studio.requestFullscreen)await studio.requestFullscreen();
  }catch{get("motion-note").textContent="Fullscreen is unavailable in this browser"};
});
document.addEventListener("fullscreenchange",()=>{get("full-label").textContent=document.fullscreenElement===studio?"Exit fullscreen":"Fullscreen";resize()});
for(const button of document.querySelectorAll("[data-power]"))button.addEventListener("click",()=>{
  const battery=button.dataset.power==="battery";
  for(const item of document.querySelectorAll("[data-power]"))item.setAttribute("aria-pressed",String(item===button));
  for(const [id,minutes] of [["lock-time",battery?2:5],["off-time",battery?3:10],["sleep-time",battery?10:30]])get(id).textContent=minutes+" min";
  get("policy-note").textContent=battery?"On battery, show the saver briefly or skip it entirely. Critical battery and lid-close sleep still take priority.":"Times count from your last input. Video playback or presentation mode can hold off automatic stages. Manual lock and lid-close sleep still work.";
});
logoAt(5);resize();updateControls();
selectMode(location.hash.slice(1));
window.addEventListener("hashchange",()=>selectMode(location.hash.slice(1)));
