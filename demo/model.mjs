import {mountModelLoader} from './model-loader.mjs';
export const MODEL_IDENTITY=Object.freeze({name:'Gemma 4 E4B',artifact:'https://huggingface.co/litert-community/gemma-4-E4B-it-litert-lm/resolve/e6a3892a8c22cf87ff99583996149cf3cb32452b/gemma-4-E4B-it-web.litertlm'});

export class LocalModel {
  // The optional presenter keeps structured state testable without a browser or GPU.
  constructor(onStatus = () => {}, onState = mountModelLoader()) {
    this.onStatus = onStatus;
    this.onState = onState;
    this.worker = null;
    this.pending = null;
    this.ready = false;
    this.statusText = '';
    this.identity = structuredClone(MODEL_IDENTITY);
  }
  notify(phase, text, details = {}) {
    this.onState({...details, phase, text});
    // Do not announce every byte update to the existing live region.
    if (text !== this.statusText) {this.statusText = text; this.onStatus(text);}
  }
  start(file = null) {
    if (this.ready) return Promise.resolve();
    this.identity = file ? {name:'Local Gemma model',artifact:null} : structuredClone(MODEL_IDENTITY);
    return this.call('load', {file}, 600000);
  }
  generate(role, prompt) {
    if (!this.ready) throw Error('Load the local model first.');
    return this.call('generate', {role, prompt}, 180000);
  }
  async call(type, payload, timeout) {
    if (this.pending) throw Error('The shared model is already busy.');
    if (type === 'load') this.notify('preparing', 'Checking your device and preparing the local runtime…');
    if (!this.worker) {
      let worker;
      // LiteRT-LM loads its WASM glue with importScripts(), which requires a classic worker.
      // model-worker.mjs uses dynamic import() for the SDK; keep that entrypoint classic-compatible.
      try {worker = new Worker(new URL('./model-worker.mjs', import.meta.url), {type: 'classic'});}
      catch (error) {this.notify('error', String(error.message || error)); throw error;}
      this.worker = worker;
      worker.onmessage = ({data}) => {
        // A terminated worker may already have queued a message. Never revive a cancelled load.
        if (this.worker !== worker || !this.pending) return;
        if (data.type === 'status') {
          const allowed = this.pending.type === 'load'
            ? ['preparing', 'downloading', 'cached', 'local', 'initializing'] : ['working'];
          if (allowed.includes(data.phase)) this.notify(data.phase, data.text, data);
          return;
        }
        if (data.error) {this.cancel(String(data.error), 'error'); return;}
        const expected = this.pending.type === 'load' ? 'loaded' : 'generated';
        if (data.type !== expected) {this.cancel('Unexpected local runtime response. Reload and retry.', 'error'); return;}
        const pending = this.pending;
        this.pending = null;
        clearTimeout(pending.timer);
        if (data.type === 'loaded') this.ready = true;
        this.notify('ready', `${this.identity.name} · ready`);
        pending.resolve(data.text);
      };
      worker.onerror = () => {
        if (this.worker === worker) this.cancel('Local runtime failed to load. Check WebGPU and network access.', 'error');
      };
    }
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => this.cancel('Local model exceeded its deadline. Reload from cache and retry.', 'error'), timeout);
      this.pending = {type, resolve, reject, timer};
      try {this.worker.postMessage({type, ...payload});}
      catch (error) {this.cancel(String(error.message || error), 'error');}
    });
  }
  cancel(message = 'Local model stopped. Reload to continue.', phase = 'stopped') {
    this.worker?.terminate();
    this.worker = null;
    this.ready = false;
    if (this.pending) {clearTimeout(this.pending.timer); this.pending.reject(Error(message)); this.pending = null;}
    this.notify(phase, message);
  }
}
