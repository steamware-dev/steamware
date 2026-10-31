(function(){
var real=window.Millennium;
var pending={},ready={},reqId=0;
function ltCall(p,m,a){
  var i='_ltr_'+(++reqId);
  pending[i]={method:m,args:a,ts:Date.now()};
  return new Promise(function(rv,rj){
    var mx=100;
    function ck(){
      var r=ready[i];
      if(r!==undefined){delete ready[i];if(r.e){rj(new Error(r.e))}else{rv(r.v)}return}
      if(--mx>0){setTimeout(ck,50)}else{delete pending[i];rv({success:true})}
    }
    ck();
  });
}
if(real&&typeof real.callServerMethod==='function'){
  var realCall=real.callServerMethod.bind(real);
  real.callServerMethod=function(p,m,a){return p==='luatools'?ltCall(p,m,a):realCall(p,m,a)};
  real._pending=pending;
  real._readyResponses=ready;
  window.Millennium=real;
}else{
  window.Millennium={_pending:pending,_readyResponses:ready,callServerMethod:function(p,m,a){return ltCall(p,m,a)}};
}
})();
