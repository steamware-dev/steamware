(function(){
  if(window.__SteamwareAddEmphasis)return;
  window.__SteamwareAddEmphasis=true;
  var ID='steamware-add-emphasis';
  var FALLBACK='rgb(102,192,244)';
  function rgba(c,a){
    var m=/rgba?\(([^)]+)\)/.exec(c||'');
    if(!m)return FALLBACK;
    var p=m[1].split(',');
    if(p.length<3)return FALLBACK;
    return 'rgba('+parseFloat(p[0])+','+parseFloat(p[1])+','+parseFloat(p[2])+','+a+')';
  }
  function accent(){
    try{
      var h=document.querySelector('button.luatools-header-button');
      if(h){
        var cs=getComputedStyle(h);
        var c=cs.borderTopColor||cs.borderColor;
        if(c&&c!=='rgba(0, 0, 0, 0)'&&c!=='transparent')return c;
      }
    }catch(e){}
    return FALLBACK;
  }
  function apply(){
    try{
      var a=accent();
      var css='a.luatools-button,.luatools-button{'+
        'border-radius:4px !important;'+
        'box-shadow:0 0 0 2px '+a+',0 0 10px '+rgba(a,0.45)+',0 2px 8px rgba(0,0,0,0.35) !important;'+
        'transition:box-shadow .18s ease,transform .18s ease !important;}'+
        'a.luatools-button:hover,.luatools-button:hover{'+
        'box-shadow:0 0 0 2px '+a+',0 0 18px '+rgba(a,0.75)+',0 5px 14px rgba(0,0,0,0.45) !important;'+
        'transform:translateY(-1px) !important;}';
      var el=document.getElementById(ID);
      if(!el){
        el=document.createElement('style');
        el.id=ID;
        (document.head||document.documentElement).appendChild(el);
      }
      if(el.textContent!==css)el.textContent=css;
    }catch(e){}
  }
  function start(){
    apply();
    // The header icon and the Add button land at different times, and the store re-renders both on SPA
    // navigation (which can also change the active palette), so the accent is re-derived on a slow
    // cadence and not just once.
    try{setTimeout(apply,400);setTimeout(apply,1500);setTimeout(apply,4000);setInterval(apply,5000);}catch(e){}
  }
  if(document.readyState==='loading')document.addEventListener('DOMContentLoaded',start);
  else start();
})();
