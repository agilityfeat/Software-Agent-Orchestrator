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
