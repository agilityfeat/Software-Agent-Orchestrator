import {validateChecks} from './page-checks.mjs';
const PREVIEW_URL=new URL('./preview/index.html',import.meta.url);
export async function createPagePreview(container,bundle,{signal,storage={},onStorage=()=>{},onError=()=>{},timeout=15000}={}){
  if(signal?.aborted)throw new DOMException('Cancelled','AbortError');
  if(!container?.append||!bundle||['html','css','js'].some(key=>typeof bundle[key]!=='string'))throw new Error('Preview needs HTML, CSS and JavaScript strings.');
  const iframe=document.createElement('iframe');iframe.setAttribute('sandbox','allow-scripts allow-forms');iframe.setAttribute('title','Application preview');iframe.style.cssText='width:100%;height:100%;border:0;display:block;background:white';iframe.referrerPolicy='no-referrer';
  const token=crypto.randomUUID();iframe.src=PREVIEW_URL.href+'?preview-token='+token;
  let disposed=false,initialized=false,loads=0,port,sequence=0,pending=new Map(),bootReject;
  const destroy=()=>{if(disposed)return;disposed=true;window.removeEventListener('message',boot);signal?.removeEventListener('abort',abort);port?.close();iframe.remove();for(const {reject,timer} of pending.values()){clearTimeout(timer);reject(new Error('Preview closed.'));}pending.clear();bootReject?.(new Error('Preview closed.'));};
  iframe.addEventListener('load',()=>{loads++;if(initialized||loads>1){onError('The app navigated away from its preview.');destroy();}});
  const abort=()=>destroy();signal?.addEventListener('abort',abort,{once:true});
  const request=(type,payload={})=>new Promise((resolve,reject)=>{if(disposed){reject(new Error('Preview closed.'));return;}const id=++sequence;const timer=setTimeout(()=>{pending.delete(id);reject(new Error('The preview did not respond. Stop or reset the page if its JavaScript is stuck.'));},timeout);pending.set(id,{resolve,reject,timer});port.postMessage({id,type,...payload});});
  let boot;
  const ready=new Promise((resolve,reject)=>{
    bootReject=reject;
    const timer=setTimeout(()=>{reject(new Error('The app preview could not start.'));destroy();},timeout);
    boot=event=>{
      if(disposed||event.source!==iframe.contentWindow||event.data?.type!=='page-preview-boot'||event.data?.token!==token)return;
      window.removeEventListener('message',boot);clearTimeout(timer);
      const channel=new MessageChannel();port=channel.port1;
      port.onmessage=event=>{
        const message=event.data;
        if(disposed||!message||typeof message!=='object')return;
        if(message.type==='storage'){onStorage(message.storage);return;}
        if(message.type==='error'){onError(message.error);return;}
        const waiting=pending.get(message.id);if(!waiting)return;pending.delete(message.id);clearTimeout(waiting.timer);if(message.error)waiting.reject(new Error(message.error));else waiting.resolve(message.result);
      };
      iframe.contentWindow.postMessage({type:'page-preview-connect',token},'*',[channel.port2]);
      request('load',{bundle,storage}).then(resolve,reject);
    };
    window.addEventListener('message',boot);container.append(iframe);
  });
  try{await ready;}catch(error){destroy();throw error;}bootReject=null;initialized=true;
  return {iframe,check:checks=>request('check',{checks:validateChecks(checks)}),inspect:()=>request('inspect'),destroy};
}
export async function runPageChecks(bundle,checks,options={}){
  const normalized=validateChecks(checks),container=document.createElement('div');container.style.cssText='position:fixed;left:-20000px;top:0;width:1200px;height:900px;overflow:hidden;pointer-events:none';container.setAttribute('aria-hidden','true');document.body.append(container);
  const rows=[],errors=[];let observations;
  try{
    // Each approved case owns its setup and gets the same clean app baseline.
    // Only actions inside that case may share state; QA cannot poison later cases.
    for(const check of normalized){
      let preview;
      try{
        preview=await createPagePreview(container,bundle,{...options,storage:{},onStorage:()=>{}});
        const result=await preview.check([check]);
        if(!Array.isArray(result.rows)||result.rows.length!==2||result.rows[0]?.name!==check.name||!Array.isArray(result.errors))throw new Error('The browser check did not return complete evidence.');
        rows.push(result.rows[0]);
        for(const error of result.errors)errors.push(check.name+': '+error);
        observations=result.observations;
      }finally{preview?.destroy();}
    }
    rows.push({name:'Page runs without browser errors',pass:errors.length===0,expected:[],actual:errors});
    return {rows,errors,observations};
  }finally{container.remove();}
}
