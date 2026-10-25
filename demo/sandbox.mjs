const active=new Set();
export function cancelAll(){for(const stop of [...active])stop(Error('Sandbox execution cancelled.'));}
export function execute(source,expenses,{timeout=15000}={}) {
  return new Promise((resolve,reject)=>{
    const worker=new Worker(new URL('./sandbox-worker.mjs',import.meta.url),{type:'module'});
    const finish=(error,value)=>{clearTimeout(timer);active.delete(finish);worker.terminate();error?reject(error):resolve(value);};
    const timer=setTimeout(()=>finish(Error('Sandbox timeout. No candidate was accepted.')),timeout);
    active.add(finish);
    worker.onmessage=({data})=>data.error?finish(Error(data.error)):finish(null,data.outputs);
    worker.onerror=()=>finish(Error('Sandbox could not load. Check CDN access and retry.'));
    worker.postMessage({source,expenses});
  });
}
