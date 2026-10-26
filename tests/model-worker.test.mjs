import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';
import {LocalModel} from '../demo/model.mjs';

// Facade assertions complement the real-worker browser regression; no GPU/provider calls.
async function withWorker(run) {
  const original = globalThis.Worker;
  const workers = [];
  globalThis.Worker = class {
    constructor(url, options) {this.url = url; this.options = options; workers.push(this);}
    postMessage() {}
    terminate() {this.stopped = true;}
    emit(data) {this.onmessage({data});}
  };
  const states = [];
  const model = new LocalModel(() => {}, event => states.push(event));
  try {await run(model, workers, states);}
  finally {
    model.cancel();
    if (original === undefined) delete globalThis.Worker;
    else globalThis.Worker = original;
  }
}

test('inference explicitly uses a classic worker at the project-relative URL', async () => {
  await withWorker(async (model, workers) => {
    const pending = model.start();
    assert.equal(workers[0].options.type, 'classic');
    assert.equal(workers[0].url.href, new URL('../demo/model-worker.mjs', import.meta.url).href);
    assert.equal(model.ready, false);
    workers[0].emit({type:'loaded'});
    await pending;
    assert.equal(model.ready, true);
  });
});

test('worker entrypoint parses as a classic script without module-only syntax', async () => {
  const source = await readFile(new URL('../demo/model-worker.mjs', import.meta.url), 'utf8');
  assert.doesNotThrow(() => new vm.Script(source));
  assert.match(source, /await import\(`/); // The SDK stays a dynamic import.
});

test('loading, initialization and a ready-looking status cannot mark the engine ready', async () => {
  await withWorker(async (model, workers, states) => {
    const pending = model.start();
    workers[0].emit({type:'status',phase:'downloading',text:'Done downloading',loaded:10,total:10,complete:true});
    workers[0].emit({type:'status',phase:'initializing',text:'Initializing'});
    workers[0].emit({type:'status',phase:'ready',text:'Not a completion'});
    assert.equal(model.ready, false);
    assert.equal(states.at(-1).phase, 'initializing');
    workers[0].emit({type:'loaded'});
    await pending;
    assert.equal(states.at(-1).phase, 'ready');
  });
});

test('one classic inference worker still serves all three role contexts', async () => {
  await withWorker(async (model, workers) => {
    const loaded = model.start(); workers[0].emit({type:'loaded'}); await loaded;
    for (const role of ['Planner','Coder','Tester']) {
      const pending = model.generate(role, 'Synthetic task');
      workers[0].emit({type:'status',phase:'working',text:'Working',role});
      workers[0].emit({type:'generated',text:role});
      assert.equal(await pending, role);
    }
    assert.equal(workers.length, 1);
    assert.equal(workers[0].options.type, 'classic');
  });
});

test('stop and retry use a fresh classic worker and ignore stale completions', async () => {
  await withWorker(async (model, workers) => {
    const first = model.start(); const stopped = assert.rejects(first, /stopped/);
    model.cancel(); await stopped;
    const second = model.start();
    workers[0].emit({type:'loaded'});
    assert.equal(model.ready, false);
    assert.equal(workers[0].stopped, true);
    assert.equal(workers[1].options.type, 'classic');
    workers[1].emit({type:'loaded'}); await second;
    assert.equal(model.ready, true);
  });
});

test('loader failures reject and never animate a ready engine', async () => {
  await withWorker(async (model, workers, states) => {
    const pending = model.start(); const failed = assert.rejects(pending, /Synthetic load error/);
    workers[0].emit({error:'Synthetic load error'}); await failed;
    assert.equal(model.ready, false);
    assert.equal(workers[0].stopped, true);
    assert.equal(states.at(-1).phase, 'error');
  });
});

test('structured Coder proposals remain data and reject ambiguous or unknown calls',async()=>{
 const source=await readFile(new URL('../demo/model-worker.mjs',import.meta.url),'utf8');
 const context=vm.createContext({self:{},URL});new vm.Script(source).runInContext(context);
 const evaluate=value=>vm.runInContext(`responseText(${JSON.stringify(value)},'Coder')`,context);
 const proposal={edits:[{file:'js',find:'"old"',replace:'"new"\nnext();'}]};
 assert.deepEqual(JSON.parse(evaluate({tool_calls:[{function:{name:'submit_app_patch',arguments:proposal}}]})),proposal);
 assert.deepEqual(JSON.parse(evaluate({tool_calls:[{function:{name:'replace_app_files',arguments:{files:{html:'<main>new</main>'}}}}]})),{files:{html:'<main>new</main>'}});
 for(const calls of [[{function:{name:'execute_code',arguments:proposal}}],[{function:{name:'submit_app_patch',arguments:'bad'}}],[{function:{name:'submit_app_patch',arguments:proposal}},{function:{name:'submit_app_patch',arguments:proposal}}]])assert.throws(()=>evaluate({tool_calls:calls}));
 assert.equal(evaluate({content:[{type:'text',text:'{"edits":[]}'}]}),'{"edits":[]}');
});


test('full-app default pins E4B and retains the existing per-URL model cache',async()=>{
 const source=await readFile(new URL('../demo/model-worker.mjs',import.meta.url),'utf8');
 assert.match(source,/gemma-4-E4B-it-litert-lm\/resolve\/e6a3892a8c22cf87ff99583996149cf3cb32452b\/gemma-4-E4B-it-web\.litertlm/);
 assert.match(source,/quota-estimate\.usage<3300000000/);
 assert.match(source,/const CACHE='agent-demo-model-v1'/);
 assert.doesNotMatch(source,/caches\.delete/);
 const {MODEL_IDENTITY}=await import('../demo/model.mjs');assert.ok(source.includes(MODEL_IDENTITY.artifact));
});

async function instrumentedWorker(stub,run,overrides={}){
 const source=await readFile(new URL('../demo/model-worker.mjs',import.meta.url),'utf8');let tick=0;
 const context=vm.createContext({self:{postMessage(){}},URL,Date,setInterval,clearInterval,performance:{now:()=>++tick*10},stub,...overrides});new vm.Script(source).runInContext(context);vm.runInContext('engine=stub',context);return run(expression=>vm.runInContext(expression,context));
}
test('generation strategy keeps production baseline and enables an explicit bounded initial-versus-repair experiment',async()=>{
 await instrumentedWorker({},evaluate=>{
  const settings=(role,options)=>JSON.parse(JSON.stringify(evaluate(`generationSettings(${JSON.stringify(role)},${JSON.stringify(options)})`)));
  assert.deepEqual(settings('Coder',{}),{strategy:'baseline',repair:false,thinking:true,maxOutputTokens:4096});
  assert.deepEqual(settings('Coder',{strategy:'fast-initial'}),{strategy:'fast-initial',repair:false,thinking:false,maxOutputTokens:2048});
  assert.deepEqual(settings('Coder',{strategy:'fast-initial',repair:true}),{strategy:'fast-initial',repair:true,thinking:true,maxOutputTokens:4096});
  assert.deepEqual(settings('Tester',{strategy:'fast-initial'}),{strategy:'baseline',repair:false,thinking:false,maxOutputTokens:2048});
  for(const options of [null,[],{strategy:'unknown'},{repair:'true'},{maxOutputTokens:999999}])assert.throws(()=>settings('Coder',options));
 });
});
test('worker telemetry reports supported benchmark counters and character counts without thought text',async()=>{
 let deleted=false,config;
 const final='{"edits":[]}';
 await instrumentedWorker({async createConversation(value){config=value;return {async sendMessage(){return {content:[{type:'text',text:final}],channels:{analysis:'private reasoning must never leave worker',other:'abc'}}},async getBenchmarkInfo(){return {lastPrefillTokenCount:123,lastPrefillTokensPerSecond:456,lastDecodeTokenCount:25,lastDecodeTokensPerSecond:12.5,timeToFirstTokenInSecond:.75}},async delete(){deleted=true}}}},async evaluate=>{
  const result=JSON.parse(JSON.stringify(await evaluate(`generateResponse({role:'Coder',prompt:'Synthetic benchmark prompt',options:{}})`)));
  assert.equal(result.text,final);assert.equal(result.metrics.outcome,'completed');assert.equal(result.metrics.finalChars,final.length);assert.equal(result.metrics.thoughtChars,'private reasoning must never leave worker'.length);assert.equal(result.metrics.otherChannelChars,3);assert.ok(result.metrics.wallMs>0);assert.ok(result.metrics.generationMs>0);assert.equal(result.metrics.benchmark.timeToFirstTokenMs,750);assert.equal(result.metrics.benchmark.decodeTokens,25);assert.equal(result.metrics.benchmark.prefillTokensPerSecond,456);assert.equal(deleted,true);assert.equal(config.preface.extra_context.enable_thinking,true);assert.equal(config.sessionConfig.maxOutputTokens,4096);assert.doesNotMatch(JSON.stringify(result),/private reasoning/);
 });
});
test('optional benchmark failure does not fail generation and failed generation still records timing and cleans up',async()=>{
 for(const fail of [false,true]){
  let deleted=false;
  await instrumentedWorker({async createConversation(){return {async sendMessage(){if(fail)throw Error('Synthetic generation failure');return {content:[{type:'text',text:'result'}]}},async getBenchmarkInfo(){throw Error('Counters unavailable')},async delete(){deleted=true}}}},async evaluate=>{
   const result=JSON.parse(JSON.stringify(await evaluate(`generateResponse({role:'Planner',prompt:'Synthetic prompt'})`)));
   assert.equal(result.metrics.outcome,fail?'error':'completed');assert.equal(deleted,true);assert.ok(result.metrics.wallMs>0);assert.ok(result.metrics.generationMs>0);assert.equal(result.metrics.benchmark,undefined);assert.equal(result.error,fail?'Synthetic generation failure':undefined);assert.equal(result.text,fail?undefined:'result');
  });
 }
});
test('LocalModel forwards optional strategy and retains bounded cloned telemetry for success and failure',async()=>{
 await withWorker(async(model,workers)=>{
  const loading=model.start();workers[0].emit({type:'loaded'});await loading;
  let sent;workers[0].postMessage=value=>sent=value;
  for(let index=0;index<52;index++){
   const pending=model.generate('Coder','Synthetic',{strategy:'fast-initial',repair:index>0});const metrics={role:'Coder',wallMs:index,outcome:'completed'};workers[0].emit({type:'generated',text:'result',metrics});await pending;metrics.wallMs=-1;
  }
  assert.deepEqual(sent.options,{strategy:'fast-initial',repair:true});assert.equal(model.metrics.length,50);assert.equal(model.lastMetrics.wallMs,51);assert.equal(model.metrics[0].wallMs,2);
  const pending=model.generate('Coder','Synthetic');const rejected=assert.rejects(pending,/Synthetic error/);workers[0].emit({error:'Synthetic error',metrics:{role:'Coder',outcome:'error',wallMs:17}});await rejected;assert.equal(model.lastMetrics.outcome,'error');assert.equal(model.metrics.length,50);
 });
});

test('real stream path waits for completion, assembles tool data and reports throttled content-free progress',async()=>{
 const messages=[],chunks=[{channels:{analysis:'secret reasoning never posted'}},{content:[{type:'text',text:''}]},{tool_calls:[{function:{name:'submit_app_patch',arguments:{edits:[{file:'html',find:'old',replace:'new'}]}}}]}];
 let index=0,clock=0,cleared=0,released=false,deleted=false;
 const stub={async createConversation(){return {sendMessage(){throw Error('Nonstream path must not run')},sendMessageStreaming(){return {getReader(){return {async read(){return index<chunks.length?{value:chunks[index++],done:false}:{done:true}},releaseLock(){released=true},async cancel(){throw Error('Completed stream must not cancel')}}}}},async getBenchmarkInfo(){return {}},async delete(){deleted=true}}}};
 await instrumentedWorker(stub,async evaluate=>{
  const result=JSON.parse(JSON.stringify(await evaluate(`generateResponse({role:'Coder',prompt:'Synthetic stream prompt'})`)));
  assert.deepEqual(JSON.parse(result.text),{edits:[{file:'html',find:'old',replace:'new'}]});assert.equal(result.metrics.receivedChunks,3);assert.equal(result.metrics.thoughtChars,'secret reasoning never posted'.length);assert.equal(result.metrics.outcome,'completed');assert.ok(result.metrics.firstResponseMs>0);assert.equal(released,true);assert.equal(deleted,true);assert.equal(cleared,1);
  const status=messages.filter(message=>message.phase==='working');assert.ok(status.length>=2);for(let i=1;i<status.length;i++)assert.ok(status[i].elapsedMs-status[i-1].elapsedMs>=1000);assert.equal(status.at(-1).receivedChunks,3);assert.match(status.at(-1).text,/Building/);assert.doesNotMatch(JSON.stringify(messages),/secret reasoning|"find"|"replace"/);
 },{self:{postMessage:value=>messages.push(value)},performance:{now:()=>clock+=600},setInterval(){return 1},clearInterval(){cleared++}});
});
test('failed stream cancels/releases its reader and clears heartbeat without exposing partial output',async()=>{
 let reads=0,cancelled=false,released=false,cleared=false,deleted=false;
 const stub={async createConversation(){return {sendMessageStreaming(){return {getReader(){return {async read(){if(reads++)throw Error('Synthetic stream failed');return {value:{content:[{type:'text',text:'partial JSON'}]},done:false}},async cancel(){cancelled=true},releaseLock(){released=true}}}}},async getBenchmarkInfo(){return {}},async delete(){deleted=true}}}};
 await instrumentedWorker(stub,async evaluate=>{const result=JSON.parse(JSON.stringify(await evaluate(`generateResponse({role:'Coder',prompt:'Synthetic'})`)));assert.equal(result.error,'Synthetic stream failed');assert.equal(result.text,undefined);assert.equal(result.metrics.outcome,'error');assert.equal(result.metrics.receivedChunks,1);assert.equal(cancelled,true);assert.equal(released,true);assert.equal(cleared,true);assert.equal(deleted,true);},{setInterval(){return 1},clearInterval(){cleared=true}});
});
test('working heartbeats cannot extend the absolute deadline or revive a stopped worker',async()=>{
 await withWorker(async(model,workers,states)=>{
  const loading=model.start();workers[0].emit({type:'loaded'});await loading;
  const native=globalThis.setTimeout;let expire,timers=0,delay;
  globalThis.setTimeout=(callback,ms)=>{expire=callback;timers++;delay=ms;return 0};
  try{
   const generating=model.generate('Coder','Synthetic');const failed=assert.rejects(generating,/3 minutes.*Load Gemma.*Start over/);const old=workers[0];
   for(const elapsedMs of [1000,90000,179000])old.emit({type:'status',phase:'working',text:'Building',role:'Coder',elapsedMs,receivedChunks:0,outputChars:0,idleMs:null});
   assert.equal(model.activity.elapsedMs,179000);assert.equal(timers,1);assert.equal(delay,180000);expire();await failed;assert.equal(model.ready,false);assert.equal(model.activity,null);assert.equal(model.pending,null);assert.equal(old.stopped,true);
   const retry=model.start();old.emit({type:'status',phase:'working',text:'stale',elapsedMs:999999});old.emit({type:'generated',text:'stale'});assert.equal(model.ready,false);assert.equal(model.activity,null);workers[1].emit({type:'loaded'});await retry;assert.equal(model.ready,true);assert.equal(states.at(-1).phase,'ready');
  }finally{globalThis.setTimeout=native;}
 });
});

test('heartbeats distinguish no first response from a stalled stream without inventing output',async()=>{
 let heartbeat,finishRead,clock=0;const messages=[];
 const stub={async createConversation(){return {sendMessageStreaming(){return {getReader(){return {read(){return new Promise(resolve=>finishRead=resolve)},async cancel(){},releaseLock(){}}}}},async getBenchmarkInfo(){return {}},async delete(){}}}};
 await instrumentedWorker(stub,async evaluate=>{
  const pending=evaluate(`generateResponse({role:'Coder',prompt:'Synthetic'})`);await new Promise(resolve=>setImmediate(resolve));
  clock=1000;heartbeat();assert.match(messages.at(-1).text,/waiting for the first response/);assert.equal(messages.at(-1).receivedChunks,0);
  finishRead({value:{content:[{type:'text',text:'{"edits":[]}'}]},done:false});await new Promise(resolve=>setImmediate(resolve));
  clock=2000;heartbeat();assert.match(messages.at(-1).text,/receiving output/);assert.equal(messages.at(-1).receivedChunks,1);
  clock=14000;heartbeat();assert.match(messages.at(-1).text,/no new output for 13s/);assert.equal(messages.at(-1).receivedChunks,1);assert.equal(messages.at(-1).idleMs,13000);
  finishRead({done:true});const result=await pending;assert.equal(result.metrics.outcome,'completed');assert.equal(result.metrics.receivedChunks,1);
 },{self:{postMessage:value=>messages.push(value)},performance:{now:()=>clock},setInterval(callback){heartbeat=callback;return 1},clearInterval(){}});
});
