(function(){
  if(window.__SteamwareRelabel)return;
  window.__SteamwareRelabel=true;
  var FROM='LuaTools',TO='Steamware';
  function fixText(n){
    var v=n&&n.nodeValue;
    if(v&&v.indexOf(FROM)>=0)n.nodeValue=v.split(FROM).join(TO);
  }
  var URLS=[['discord.gg/luatools','discord.gg/steamtools']];
  function fixAttrs(e){
    if(!e||e.nodeType!==1||!e.getAttribute)return;
    // 'alt' belongs here: the plugin labels its own title icon with the brand (titleIcon.alt = the old
    // product name), which is what a screen reader and a failed image load surface. Missing it left that
    // one visible string on the old name while every text node around it already read Steamware.
    var attrs=['title','aria-label','placeholder','alt'];
    for(var i=0;i<attrs.length;i++){
      var val=e.getAttribute(attrs[i]);
      if(val&&val.indexOf(FROM)>=0)e.setAttribute(attrs[i],val.split(FROM).join(TO));
    }
  }
  // Links the plugin opens itself (the in-menu Discord button) are plain URLs, not brand text, so they
  // get their own map: wherever the URL lives (href/src/data-url or an inline handler) it is repointed.
  function fixUrls(e){
    if(!e||e.nodeType!==1||!e.getAttribute)return;
    var attrs=['href','src','data-url','data-href','onclick'];
    for(var i=0;i<attrs.length;i++){
      var v=e.getAttribute(attrs[i]);
      if(!v)continue;
      for(var j=0;j<URLS.length;j++){
        if(v.indexOf(URLS[j][0])>=0)e.setAttribute(attrs[i],v.split(URLS[j][0]).join(URLS[j][1]));
      }
    }
  }
  function fixEl(e){fixAttrs(e);fixUrls(e);}
  function walk(root){
    if(!root)return;
    if(root.nodeType===3){fixText(root);return;}
    if(root.nodeType!==1)return;
    fixEl(root);
    var w=document.createTreeWalker(root,NodeFilter.SHOW_TEXT,null),n;
    while((n=w.nextNode()))fixText(n);
    var els=root.querySelectorAll?root.querySelectorAll('*'):[];
    for(var j=0;j<els.length;j++){
      fixEl(els[j]);
      if(els[j].shadowRoot)walk(els[j].shadowRoot);
    }
  }
  function sweep(){
    // Backgrounded tabs are skipped: nothing is on screen to fix and the full walk is the costly part.
    if(document.hidden)return;
    walk(document.body);
    var all=document.querySelectorAll('*');
    for(var i=0;i<all.length;i++)if(all[i].shadowRoot)walk(all[i].shadowRoot);
  }
  function start(){
    sweep();
    var mo=new MutationObserver(function(muts){
      for(var i=0;i<muts.length;i++){
        var m=muts[i];
        if(m.type==='characterData'){fixText(m.target);continue;}
        if(m.type==='attributes'){fixUrls(m.target);continue;}
        if(m.addedNodes)for(var k=0;k<m.addedNodes.length;k++)walk(m.addedNodes[k]);
      }
    });
    mo.observe(document.documentElement,{childList:true,subtree:true,characterData:true,attributes:true,attributeFilter:['href','src','data-url','data-href']});
    // Late/async renders (the Add button, the plugin menu) land after DOMContentLoaded, so a few one-shot
    // sweeps cover them; the observer handles everything after that. This replaces the old 1.5s
    // full-document walk, which was needless work on a busy store page.
    try{setTimeout(sweep,300);setTimeout(sweep,1200);setTimeout(sweep,3000);setInterval(sweep,6000);}catch(e){}
  }
  if(document.readyState==='loading')document.addEventListener('DOMContentLoaded',start);
  else start();
})();
