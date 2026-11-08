// Original Canvas studies. Inspiration and component links are listed in README.md.
globalThis.desSaverScenes = (() => {
  const tau = Math.PI * 2;
  const mix = (a,b,p) => a + (b-a)*p;
  const clamp = n => Math.max(0, Math.min(n, 1));
  const frac = n => n-Math.floor(n);
  const rand = n => frac(Math.sin((n+1)*127.164)*43758.5453);
  const rgba = (c,a) => `rgba(${c},${clamp(a)})`;
  const dot = (ctx,x,y,r) => {ctx.beginPath();ctx.arc(x,y,r,0,tau);ctx.fill()};
  function glow(ctx,x,y,r,c,a) {
    if(r<=0)return;
    const g=ctx.createRadialGradient(x,y,0,x,y,r);
    g.addColorStop(0,rgba(c,a));g.addColorStop(.38,rgba(c,a*.31));g.addColorStop(1,rgba(c,0));
    ctx.fillStyle=g;ctx.fillRect(x-r,y-r,r*2,r*2);
  }
  function ground(ctx,w,h,top,bottom) {
    const g=ctx.createLinearGradient(0,0,0,h);
    g.addColorStop(0,top);g.addColorStop(1,bottom);
    ctx.fillStyle=g;ctx.fillRect(0,0,w,h);
  }
  function stars(ctx,w,h,t,n,ceiling=1) {
    for(let i=0;i<n;i++){
      const x=rand(i*6)*w,y=rand(i*6+1)*h*ceiling;
      const a=(.2+rand(i*6+2)*.62)*(.84+.16*Math.sin(t*.23+i));
      ctx.fillStyle=rgba(i%8===0?"160,203,236":"214,230,244",a);
      dot(ctx,x,y,.3+rand(i*6+3)*.85);
    }
  }
  // The glow field is intentionally low resolution; the mountain silhouettes and
  // wordmark are still drawn at full canvas resolution.
  let auroraLayer;
  const auroraStops=[[43,34,143],[45,113,170],[75,216,170],[103,64,191]];
  function paintAurora(ctx,w,h,t,p,px,py,energy){
    if(!auroraLayer){
      const canvas=document.createElement("canvas");canvas.width=256;canvas.height=100;
      const paint=canvas.getContext("2d");
      auroraLayer={canvas,paint,image:paint.createImageData(canvas.width,canvas.height)};
    }
    const {canvas,paint,image}=auroraLayer,{width,height}=image;
    const {data}=image;
    for(let x=0;x<width;x++){
      const u=x/(width-1);
      const drift=Math.sin(u*5.2-t*.21)*.06+Math.sin(u*12.7+t*.31)*.034;
      const hem=.52+drift+Math.sin(u*2.6+t*.17)*.06+(py-.5)*energy*.08*Math.exp(-18*(u-px)**2);
      const stop=clamp(u+.035*Math.sin(t*.23+u*4))*3,lo=Math.min(2,Math.floor(stop));
      const tint=auroraStops[lo].map((v,i)=>mix(v,auroraStops[lo+1][i],stop-lo));
      const wisps=.91+.09*Math.sin(u*75+t*.26+2*Math.sin(u*11-t*.19));
      for(let y=0;y<height;y++){
        const v=y/(height-1),d=(v-hem)/.105;
        const shade=1/(1+Math.exp(d*2.9));
        const second=1/(1+Math.exp((d+.43*Math.sin(u*9+t*.3)-.4)*2.25));
        const glow=shade*(.35+.31*p)+second*(.10+.14*p);
        const i=(y*width+x)*4;
        data[i]=tint[0]*.91;
        data[i+1]=tint[1];
        data[i+2]=tint[2];
        data[i+3]=255*glow*wisps*(.73+.27*Math.sin(v*3.2+u*1.4));
      }
    }
    paint.putImageData(image,0,0);
    ctx.drawImage(canvas,0,0,w,h);
  }

  function polar({ctx,w,h,t,p,px,py,energy,brand}) {
    ground(ctx,w,h,"#040713","#07131c");
    stars(ctx,w,h,t,112,.83);
    paintAurora(ctx,w,h,t,p,px,py,energy);
    for(let j=0;j<3;j++){
      ctx.fillStyle=["#101b2a","#0b1623","#070e18"][j];ctx.beginPath();ctx.moveTo(0,h);
      for(let x=0;x<=w+12;x+=12){
        const u=x/w, teeth=Math.abs(Math.sin(u*18+j*1.52)+.46*Math.sin(u*39+j*2.71));
        ctx.lineTo(x,h*(.86+j*.052-teeth*(.09-j*.019)));
      }
      ctx.lineTo(w,h);ctx.closePath();ctx.fill();
    }
    brand(w*.5,h*.76,Math.min(w*.24,265),"#b7e3d6",.64);
  }

  function eclipse({ctx,w,h,t,p,px,py,energy,brand}) {
    ground(ctx,w,h,"#050309","#0a050d");stars(ctx,w,h,t,118);
    const r=Math.min(w*.23,h*.307), x=w*(.5+(px-.5)*.044*energy),y=h*(.48+(py-.5)*.046*energy);
    glow(ctx,x,y,r*2.5,"128,28,85",.31+.27*p);
    glow(ctx,x+r*.70,y-r*.55,r*1.55,"206,72,56",.26+.31*p);
    ctx.globalCompositeOperation="screen";
    const colors=["244,78,70","246,110,106","180,78,190","255,174,119"];
    for(let j=0;j<5;j++){
      for(let i=0;i<100;i++){
        const a=i/100*tau, k=a+t*.24+j*.53;
        const hot=Math.pow((1+Math.cos(a+.68-t*.12))*.5,4);
        const rad=r*(1.025+j*.035+.013*Math.sin(k*7+j));
        const x0=x+Math.cos(a)*rad,y0=y+Math.sin(a)*rad;
        const len=r*(.02+.26*hot)*rand(i*11+j+1);
        ctx.strokeStyle=rgba(colors[j%colors.length],(.045+hot*.27)*(1+p*.7));
        ctx.lineWidth=.8+j*.25;ctx.beginPath();ctx.moveTo(x0,y0);
        ctx.quadraticCurveTo(x0+Math.cos(a+.32)*len*.6,y0+Math.sin(a+.32)*len*.6,x0+Math.cos(a+.59)*len,y0+Math.sin(a+.59)*len);ctx.stroke();
      }
    }
    ctx.globalCompositeOperation="source-over";
    const disc=ctx.createRadialGradient(x-r*.34,y+r*.36,0,x,y,r*1.06);
    disc.addColorStop(0,"#130d1c");disc.addColorStop(.78,"#07060b");disc.addColorStop(1,"#040306");
    ctx.fillStyle=disc;dot(ctx,x,y,r);
    const rim=ctx.createLinearGradient(x-r,y+r,x+r,y-r);
    rim.addColorStop(0,"#50204c");rim.addColorStop(.38,"#aa3d78");rim.addColorStop(.76,"#ff906e");rim.addColorStop(1,"#ffe5ba");
    ctx.strokeStyle=rim;ctx.lineWidth=Math.max(1,r*.018);ctx.beginPath();ctx.arc(x,y,r,0,tau);ctx.stroke();
    glow(ctx,x+r*.71,y-r*.71,r*.42,"255,162,116",.62+p*.27);
    brand(x,y,Math.min(r*1.21,w*.23),"#e3bed6",.65);
  }

  function transit({ctx,w,h,t,p,px,energy,brand}) {
    ground(ctx,w,h,"#08051a","#08050d");
    const y0=h*.46, shift=(px-.5)*.075*energy, cx=w*(.52+shift+.04*Math.sin(t*.2));
    glow(ctx,cx,y0,h*.58,"203,37,107",.46);
    stars(ctx,w,h,t,76,.47);
    const horizon=ctx.createLinearGradient(cx,0,cx,y0);
    horizon.addColorStop(0,"#24113b");horizon.addColorStop(1,"#0d091c");
    ctx.fillStyle=horizon;
    for(let i=0;i<60;i++){
      const bx=i*w/60, bw=w/59, bh=(.014+.101*rand(i*7))*h;
      ctx.fillRect(bx,y0-bh,bw-1,y0-bh>0?bh:0);
      ctx.save();ctx.fillStyle=rgba(i%3===0?"255,88,147":"90,144,181",.36);
      for(let j=0;j<4;j++)if(rand(i*17+j)>.63)ctx.fillRect(bx+bw*.25,y0-bh*.26-j*bh*.12,Math.max(1,bw*.11),1.4);
      ctx.restore();
    }
    const pos=(lane,z)=>[cx+w*(lane*.77*z+.18*z*z*Math.sin(t*.22+.62+z*1.6)),y0+z*z*h*.61];
    const road=ctx.createLinearGradient(0,y0,0,h);road.addColorStop(0,"#120b19");road.addColorStop(1,"#19121e");
    ctx.fillStyle=road;ctx.beginPath();
    for(let edge=0;edge<2;edge++)for(let j=0;j<=40;j++){
      const q=pos(edge? .87:-.87,edge?1-j/40:j/40);
      if(!edge&&!j)ctx.moveTo(...q);else ctx.lineTo(...q);
    }
    ctx.closePath();ctx.fill();
    for(const lane of [-.85,-.4,0,.4,.85]){
      ctx.lineWidth=lane===0?1.1:1.6;ctx.strokeStyle=lane<0?"#75364e":lane>0?"#31576d":"#4b364a";
      ctx.beginPath();
      for(let j=0;j<=40;j++){const q=pos(lane,j/40);if(j)ctx.lineTo(...q);else ctx.moveTo(...q);}
      ctx.stroke();
    }
    ctx.globalCompositeOperation="screen";
    for(let i=0;i<110;i++){
      const lane=(i%2?1:-1)*(.12+.58*rand(i*5+2)), speed=.064+p*.13;
      const z=frac(rand(i*5)+t*speed*(.65+rand(i*5+1)*.51));
      const tail=Math.max(0,z-(.034+p*.13)*(.55+z)*rand(i*5+3));
      const c=lane<0?(i%3===0?"255,178,96":"255,53,113"):(i%3===0?"151,246,255":"63,161,214");
      ctx.lineCap="round";
      for(let pass=0;pass<2;pass++){
        ctx.lineWidth=(.25+z*2.3)*(pass===0?4.4:1);
        ctx.strokeStyle=rgba(c,(pass===0?.20:.77)*Math.min(1,z*2.5));
        ctx.beginPath();
        for(let j=0;j<=7;j++){const q=pos(lane,mix(tail,z,j/7));if(j)ctx.lineTo(...q);else ctx.moveTo(...q);}
        ctx.stroke();
      }
    }
    ctx.lineCap="butt";ctx.globalCompositeOperation="source-over";
    brand(w*.5,h*.23,Math.min(w*.25,262),"#f4afcb",.73);
  }

  function prism({ctx,w,h,t,p,px,py,energy,brand}) {
    ground(ctx,w,h,"#0b0717","#090812");
    glow(ctx,w*.26,h*.47,w*.53,"71,27,130",.33);
    glow(ctx,w*.74,h*.43,w*.48,"30,116,104",.21);
    const mag=(px-.5)*energy*.16, lift=(py-.5)*energy*.12;
    const ramps=[
      ["#160c29","#241939","#634b9a","#c982d9","#372146","#100b21"],
      ["#132326","#264f58","#70b5ae","#caebcf","#453755","#18152c"],
      ["#1c0e29","#41204b","#ad5c94","#ffc3a7","#704474","#1e1233"],
      ["#14152a","#293863","#5f68ad","#b4cafa","#3c425b","#131222"],
      ["#1e1224","#4e2749","#c274a3","#ffd0ad","#473a65","#211538"]
    ];
    for(let j=0;j<5;j++){
      const phase=j*.81+t*.18;
      const at=(u)=>h*(.1+j*.163+Math.sin(u*5.4+phase)*(.195-j*.014)+.07*Math.sin(u*11.9-t*.25+j*.7)+lift);
      const thickness=(u)=>h*(.046+.045*(.5+.5*Math.sin(u*4.7+j*.9-t*.17))+.009*j);
      const palette=ramps[j];
      for(let n=0;n<21;n++){
        const f=n/20, a=clamp(.56+p*.38);
        const tone=Math.min(palette.length-1,Math.floor(f*palette.length));
        ctx.lineWidth=Math.max(1.3,h/166);
        ctx.strokeStyle=palette[tone];ctx.globalAlpha=a*(.73+.27*Math.sin(f*Math.PI));
        ctx.beginPath();
        for(let x=-10;x<=w+10;x+=8){
          const u=x/w+mag,y=at(u)+thickness(u)*(f-.5);
          if(x===-10)ctx.moveTo(x,y);else ctx.lineTo(x,y);
        }
        ctx.stroke();
      }
      ctx.globalAlpha=.22+.18*p;
      ctx.lineWidth=1;ctx.strokeStyle=palette[3];ctx.beginPath();
      for(let x=-10;x<=w+10;x+=8){
        const u=x/w+mag,y=at(u)+thickness(u)*.20;
        if(x===-10)ctx.moveTo(x,y);else ctx.lineTo(x,y);
      }
      ctx.stroke();
    }
    ctx.globalAlpha=1;
    const veil=ctx.createRadialGradient(w*.5,h*.52,h*.04,w*.5,h*.52,w*.24);
    veil.addColorStop(0,"rgba(9,7,18,.90)");veil.addColorStop(.48,"rgba(9,7,18,.65)");veil.addColorStop(1,"rgba(9,7,18,0)");
    ctx.fillStyle=veil;ctx.fillRect(0,0,w,h);
    brand(w*.5,h*.52,Math.min(w*.32,366),"#e8c6f3",.8);
  }

  function radar({ctx,w,h,t,p,px,py,energy,brand}) {
    ground(ctx,w,h,"#07110f","#050a0a");
    const x=w*.5,y=h*.49,r=Math.min(w*.34,h*.389);
    const angle=frac(t*.108+.17)*tau-Math.PI/2;
    glow(ctx,x,y,r*2.4,"34,90,66",.28);
    ctx.save();ctx.beginPath();ctx.arc(x,y,r,0,tau);ctx.clip();
    ctx.fillStyle="#081b14";ctx.fillRect(x-r,y-r,r*2,r*2);
    ctx.lineWidth=1;ctx.strokeStyle="#194333";
    for(let k=-6;k<=6;k++){
      const z=k*r/6;ctx.beginPath();ctx.moveTo(x+z,y-r);ctx.lineTo(x+z,y+r);ctx.moveTo(x-r,y+z);ctx.lineTo(x+r,y+z);ctx.stroke();
    }
    ctx.strokeStyle="#31644f";
    for(let j=1;j<=4;j++){ctx.beginPath();ctx.arc(x,y,r*j/4,0,tau);ctx.stroke();}
    for(let i=0;i<38;i++){
      const thickness=.020+p*.012, a=angle-i*thickness;
      ctx.fillStyle=rgba("91,236,155",(.31+p*.21)*Math.exp(-i/(8+p*13)));
      ctx.beginPath();ctx.moveTo(x,y);ctx.arc(x,y,r,a-thickness,a+.003);ctx.closePath();ctx.fill();
    }
    ctx.strokeStyle="#8bffbf";ctx.lineWidth=1.8;ctx.beginPath();ctx.moveTo(x,y);ctx.lineTo(x+r*Math.cos(angle),y+r*Math.sin(angle));ctx.stroke();
    for(let i=0;i<16;i++){
      const a=tau*rand(i*9)+Math.sin(t*.29+i)*.12;
      const d=r*(.18+rand(i*9+2)*.71);
      const decay=Math.exp(-frac((angle-a)/tau)*(6-p*2.6));
      const tx=x+Math.cos(a)*d,ty=y+Math.sin(a)*d;
      ctx.fillStyle=rgba(i%5===0?"250,209,130":"139,253,180",.09+decay*.84);dot(ctx,tx,ty,1.2+decay*1.8);
      if(decay>.60){
        ctx.strokeStyle=rgba("136,245,174",decay*.44);ctx.strokeRect(tx-4,ty-4,8,8);
      }
    }
    const tx=px*w,ty=py*h;
    if(energy>.03&&Math.hypot(tx-x,ty-y)<r*.97){
      ctx.strokeStyle=rgba("172,255,195",energy*.62);ctx.lineWidth=1;ctx.beginPath();ctx.arc(tx,ty,10+(1-energy)*13,0,tau);ctx.stroke();
    }
    ctx.restore();
    ctx.lineWidth=1;ctx.strokeStyle="#4a916e";ctx.beginPath();ctx.arc(x,y,r+1,0,tau);ctx.stroke();
    for(let i=0;i<60;i++){
      const a=i/60*tau,len=i%5===0?8:4;
      ctx.strokeStyle=i%5===0?"#6aac86":"#315842";
      ctx.beginPath();ctx.moveTo(x+(r+5)*Math.cos(a),y+(r+5)*Math.sin(a));ctx.lineTo(x+(r+5+len)*Math.cos(a),y+(r+5+len)*Math.sin(a));ctx.stroke();
    }
    brand(x,y+r+26,Math.min(w*.21,r*1.11),"#8bcba6",.62);
    if(w>680){
      ctx.font="10px JetBrains, monospace";ctx.fillStyle="#609776";ctx.fillText("PASSIVE SCAN",w*.065,h*.29);
      ctx.fillStyle="#385d4b";ctx.fillText("SECTOR / 01",w*.065,h*.29+20);
      ctx.textAlign="right";ctx.fillText("360° / SWEEP",w*.94,h*.70);ctx.textAlign="left";
    }
  }

  return {
    "polar-veil": {name:"Polar Veil", description:"A soft violet, blue and mint aurora fades above a quiet mountain horizon.", control:"Glow", hint:"Move the pointer to lean the aurora", level:.55, draw:polar},
    "event-horizon": {name:"Event Horizon", description:"An ember corona crawls around a dark eclipse in a quiet star field.", control:"Corona", hint:"Move the pointer to shift the eclipse", level:.55, draw:eclipse},
    "night-transit": {name:"Night Transit", description:"Rose and cyan traffic trails trace a highway into a neon horizon.", control:"Speed", hint:"Move the pointer to steer the view", level:.47, draw:transit},
    "liquid-prism": {name:"Liquid Prism", description:"Bands of rose, jade and lilac glass slide past the DEs-UI wordmark.", control:"Sheen", hint:"Move the pointer to bend the ribbons", level:.62, draw:prism},
    "phosphor-sweep": {name:"Phosphor Sweep", description:"A green radar sweep leaves fading contacts and a phosphor trail.", control:"Trail", hint:"Move the pointer over the scope to mark a contact", level:.56, draw:radar}
  };
})();
