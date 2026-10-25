export class LocalModel {
  constructor(onStatus=()=>{}) {this.onStatus=onStatus;this.worker=null;this.pending=null;this.ready=false;}
  start(file=null) {return this.call('load',{file},600000);}
  generate(role,prompt) {if(!this.ready)throw Error('Load the local model first.');return this.call('generate',{role,prompt},180000);}
  async call(type,payload,timeout) {
    if(this.pending)throw Error('The shared model is already busy.');
    if(!this.worker){
      this.worker=new Worker(new URL('./model-worker.mjs',import.meta.url),{type:'module'});
      this.worker.onmessage=({data})=>{
        if(data.type==='status'){this.onStatus(data.text);return;}
        if(!this.pending)return;
        const pending=this.pending;this.pending=null;clearTimeout(pending.timer);
        if(data.error){this.ready=false;this.worker.terminate();this.worker=null;this.onStatus('Model unavailable');pending.reject(Error(data.error));}
        else {if(data.type==='loaded')this.ready=true;this.onStatus('Gemma 4 E2B · ready');pending.resolve(data.text);}
      };
      this.worker.onerror=()=>this.cancel('Local runtime failed to load. Check WebGPU and network access.');
    }
    return new Promise((resolve,reject)=>{
      const timer=setTimeout(()=>this.cancel('Local model exceeded its deadline. Reload from cache and retry.'),timeout);
      this.pending={resolve,reject,timer};this.worker.postMessage({type,...payload});
    });
  }
  cancel(message='Local model stopped. Reload to continue.') {
    this.worker?.terminate();this.worker=null;this.ready=false;
    if(this.pending){clearTimeout(this.pending.timer);this.pending.reject(Error(message));this.pending=null;}
    this.onStatus(message);
  }
}
