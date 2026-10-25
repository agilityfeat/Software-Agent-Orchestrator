import test, {beforeEach, afterEach} from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {loaderPresentation, mountModelLoader} from '../demo/model-loader.mjs';
import {LocalModel} from '../demo/model.mjs';

// Synthetic worker events exercise the real facade. No model download, GPU or provider calls.
const originalWorker = globalThis.Worker;
class FakeWorker {
  static instances = [];
  constructor(url, options) {this.url = url; this.options = options; this.messages = []; FakeWorker.instances.push(this);}
  postMessage(message) {this.messages.push(message);}
  terminate() {this.terminated = true;}
  emit(data) {this.onmessage({data});}
}
beforeEach(() => {globalThis.Worker = FakeWorker; FakeWorker.instances = [];});
afterEach(() => {if (originalWorker === undefined) delete globalThis.Worker; else globalThis.Worker = originalWorker;});
const setup = () => {
  const states = [], messages = [];
  return {states, messages, model: new LocalModel(text => messages.push(text), state => states.push(state))};
};
const latest = () => FakeWorker.instances.at(-1);

test('headless presenter is a no-op', () => assert.doesNotThrow(() => mountModelLoader(null)({phase:'ready'})));
test('initial state is asleep, not ready', () => {const view = loaderPresentation(); assert.equal(view.badge,'ASLEEP'); assert.equal(view.loading,false);});
test('download reports actual byte fraction', () => {const view = loaderPresentation({phase:'downloading',loaded:500000000,total:2000000000}); assert.equal(view.percent,25); assert.match(view.detail,/25%/); assert.match(view.detail,/0.50|500.0/);});
test('unknown total does not invent a percentage', () => {const view = loaderPresentation({phase:'downloading',loaded:800000000}); assert.equal(view.percent,null); assert.match(view.detail,/downloaded/); assert.doesNotMatch(view.detail,/%/);});
test('invalid or overrun totals remain indeterminate', () => {for (const total of [null,0,-5,Infinity,NaN,5]) assert.equal(loaderPresentation({phase:'downloading',loaded:10,total}).percent,null);});
test('100% download is not a ready engine', () => {const view = loaderPresentation({phase:'downloading',loaded:10,total:10,complete:true}); assert.equal(view.percent,100); assert.equal(view.badge,'WAKING UP');});
test('unfinished stream never displays 100%', () => assert.equal(loaderPresentation({phase:'downloading',loaded:10,total:10}).percent,99));
test('cached GPU initialization stays indeterminate and not ready', () => {const view = loaderPresentation({phase:'initializing',source:'cache'}); assert.equal(view.percent,null); assert.equal(view.loading,true); assert.match(view.detail,/Browser cache/); assert.match(view.detail,/not ready yet/);});
test('errors and cancellation offer retry without active loading', () => {for (const phase of ['error','stopped']) {const v=loaderPresentation({phase,text:'Synthetic failure'}); assert.equal(v.loading,false); assert.equal(v.button,'Retry loading Gemma'); assert.equal(v.detail,'Synthetic failure');}});
test('ready and working clearly identify the loaded local model', () => {assert.equal(loaderPresentation({phase:'ready'}).badge,'✓ READY'); assert.match(loaderPresentation({phase:'working',role:'Coder'}).title,/Coder/);});

test('ready requires the actual loaded completion, not progress or status prose', async () => {
  const {model, states} = setup(); const pending=model.start(); const w=latest();
  assert.equal(states[0].phase,'preparing'); assert.equal(model.ready,false);
  w.emit({type:'status',phase:'downloading',text:'Downloading',loaded:10,total:10,complete:true});
  w.emit({type:'status',phase:'initializing',text:'Initializing'});
  w.emit({type:'status',phase:'ready',text:'Spoofed ready status'});
  assert.equal(model.ready,false); assert.equal(states.at(-1).phase,'initializing');
  w.emit({type:'loaded'}); await pending;
  assert.equal(model.ready,true); assert.equal(states.at(-1).phase,'ready');
});
test('progress does not repeatedly announce the same status text', async () => {
  const {model,messages,states}=setup(); const p=model.start();
  for(let loaded=0;loaded<5;loaded++) latest().emit({type:'status',phase:'downloading',text:'Downloading',loaded,total:10});
  assert.equal(messages.filter(x=>x==='Downloading').length,1); assert.equal(states.filter(x=>x.phase==='downloading').length,5);
  latest().emit({type:'loaded'}); await p;
});
test('cache and local-file starts share the same completion gate', async () => {
  for (const phase of ['cached','local']) {
    const {model,states}=setup();const file=phase==='local'?{synthetic:true}:null;const p=model.start(file);const w=latest();
    assert.equal(w.messages[0].file,file);w.emit({type:'status',phase,text:'Source found',source:phase==='cached'?'cache':'file'});
    assert.equal(model.ready,false);assert.equal(states.at(-1).phase,phase);w.emit({type:'loaded'});await p;
  }
});
test('generation retains the engine and does not download again', async () => {
  const {model,states}=setup();const load=model.start();const w=latest();w.emit({type:'loaded'});await load;
  const generate=model.generate('Coder','Synthetic task');w.emit({type:'status',phase:'working',text:'Coding locally',role:'Coder'});
  assert.equal(model.ready,true);assert.equal(states.at(-1).phase,'working');w.emit({type:'generated',text:'function code() {}'});
  assert.equal(await generate,'function code() {}');assert.equal(states.at(-1).phase,'ready');assert.equal(FakeWorker.instances.length,1);
});
test('failed load dims the bot and rejects instead of declaring ready', async () => {
  const {model,states}=setup();const load=model.start();const failure=assert.rejects(load,/Synthetic failure/);const w=latest();
  w.emit({error:'Synthetic failure'});await failure;assert.equal(model.ready,false);assert.equal(states.at(-1).phase,'error');assert.equal(w.terminated,true);
});
test('cancelled workers and their late messages cannot revive the bot or a retry', async () => {
  const {model,states}=setup();const first=model.start();const failed=assert.rejects(first,/stopped/);const old=latest();
  model.cancel();await failed;assert.equal(states.at(-1).phase,'stopped');assert.equal(old.terminated,true);
  const second=model.start();const current=latest();old.emit({type:'loaded'});old.emit({type:'status',phase:'initializing',text:'stale'});
  assert.equal(model.ready,false);assert.equal(states.at(-1).phase,'preparing');current.emit({type:'loaded'});await second;assert.equal(model.ready,true);
});
test('duplicate start does not reset progress or create another worker', async () => {
  const {model,states}=setup();const p=model.start();latest().emit({type:'status',phase:'downloading',text:'Downloading',loaded:10,total:20});
  await assert.rejects(model.start(),/already busy/);assert.equal(states.at(-1).phase,'downloading');assert.equal(FakeWorker.instances.length,1);
  latest().emit({type:'loaded'});await p;
});
test('already loaded start is a no-op', async () => {const {model}=setup();const p=model.start();latest().emit({type:'loaded'});await p;await model.start();assert.equal(latest().messages.length,1);});
test('worker construction failure is visible and leaves no pending operation', async () => {
  globalThis.Worker=class {constructor(){throw Error('Synthetic worker setup failure');}};
  const {model,states}=setup();await assert.rejects(model.start(),/setup failure/);assert.equal(states.at(-1).phase,'error');assert.equal(model.pending,null);
});
test('worker transport failure clears pending timers and permits retry', async () => {
  const {model,states}=setup();const p=model.start();const failed=assert.rejects(p,/runtime failed/);latest().onerror();await failed;
  assert.equal(states.at(-1).phase,'error');assert.equal(model.pending,null);assert.equal(model.ready,false);
});
test('unexpected completion cannot mark a load ready', async () => {
  const {model}=setup();const p=model.start();const fail=assert.rejects(p,/Unexpected/);latest().emit({type:'generated',text:'not a load'});await fail;assert.equal(model.ready,false);
});
test('timeout is an error state, not a success animation', async () => {
  const {model,states}=setup();const native=globalThis.setTimeout;let expire;
  globalThis.setTimeout=callback=>{expire=callback;return 0;};
  try {const p=model.start();const failed=assert.rejects(p,/deadline/);expire();await failed;assert.equal(states.at(-1).phase,'error');assert.equal(model.ready,false);}
  finally {globalThis.setTimeout=native;}
});
test('animation supports both reduced-motion and a manual pause', async () => {
  const css=await readFile(new URL('../demo/model-loader.css',import.meta.url),'utf8');
  assert.match(css,/prefers-reduced-motion:reduce/);assert.match(css,/animation:none!important/);assert.match(css,/animation-play-state:paused!important/);assert.match(css,/@keyframes bot-wave/);
});
test('worker exposes typed phases and waits for engine creation before loaded', async () => {
  const source=await readFile(new URL('../demo/model-worker.mjs',import.meta.url),'utf8');
  assert.match(source,/phase,text,\.\.\.details/);assert.match(source,/await Engine.create/);assert.match(source,/await loadModel\(data.file\);self.postMessage\(\{type:'loaded'\}\)/);
  assert.doesNotMatch(source,/\|\|2010000000/);
});
