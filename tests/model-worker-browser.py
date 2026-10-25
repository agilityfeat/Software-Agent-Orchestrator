#!/usr/bin/env python3
"""Real Chromium worker regression (no model download).

Run: python3 tests/model-worker-browser.py
Requires Playwright + Chromium, as documented in demo/README.md.
The default uses production worker/facade bodies with an explicitly synthetic
SDK/GPU fixture. Native Worker, dynamic import and importScripts are NOT mocked.
--runtime additionally initializes the actual pinned CDN SDK/WASM (no weights).
A passing fixture alone does not validate CDN access, WebGPU or Gemma inference.
"""
import argparse
import json
import pathlib
import re
import shutil
from playwright.sync_api import sync_playwright

ROOT = pathlib.Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--runtime', action='store_true', help='also load real pinned SDK/WASM; requires network')
args = parser.parse_args()
sources = {name: (ROOT / 'demo' / name).read_text() for name in ('model.mjs', 'model-worker.mjs', 'model-loader.mjs')}
sdk_match = re.search(r"const SDK='([^']+)'", sources['model-worker.mjs'])
if not sdk_match:
    raise RuntimeError('Cannot locate pinned SDK; update the test explicitly.')

HARNESS = r'''async ({sources, sdk, mode}) => {
  // Self-contained data URLs work for both worker types on an in-memory page.
  const scriptUrl = source => 'data:text/javascript;charset=utf-8,' + encodeURIComponent(source);
  const replaceOnce = (source, from, to) => {
    if (source.split(from).length !== 2) throw Error('Test wiring changed: ' + from);
    return source.replace(from, to);
  };
  let model, timer;
  try {
    let workerSource;
    if (mode === 'runtime') {
      // Actual SDK/WASM initialization only: never requests model weights.
      workerSource = `self.onmessage = async () => {
        try {
          const {loadLiteRtLm} = await import(${JSON.stringify(sdk + '/+esm')});
          await loadLiteRtLm(${JSON.stringify(sdk + '/wasm/')});
          self.postMessage({type:'loaded'});
        } catch (error) {self.postMessage({error:String(error.message || error)});}
      };`;
    } else {
      const glue = scriptUrl('self.__syntheticGlueLoaded = true;');
      const fixtureSdk = scriptUrl(`
        export async function loadLiteRtLm(path) {
          if (path !== ${JSON.stringify(sdk + '/wasm/')}) throw Error('Unexpected WASM path');
          // The browser performs this call. A module worker must fail here.
          importScripts(${JSON.stringify(glue)});
          if (!self.__syntheticGlueLoaded) throw Error('Glue not executed');
        }
        export const Engine = {async create() {
          if (!self.__syntheticGlueLoaded) throw Error('Engine before runtime');
          return {async createConversation() {return {
            async sendMessage() {return {content:[{type:'text',text:'synthetic reply'}]};},
            async delete() {}
          };}};
        }};
      `);
      workerSource = replaceOnce(sources['model-worker.mjs'],
        'await import(`${SDK}/+esm`)', `await import(${JSON.stringify(fixtureSdk)})`);
      // Only the test fixture gets a fake GPU and a fake model file. No network fallback.
      workerSource = `Object.defineProperty(navigator, 'gpu', {value:{requestAdapter:async()=>({})}});
        self.fetch = () => {throw Error('Unexpected network request in fixture');};\n` + workerSource;
    }
    const workerUrl = scriptUrl(workerSource);
    const presenterUrl = scriptUrl(sources['model-loader.mjs']);
    let facade = replaceOnce(sources['model.mjs'], "'./model-loader.mjs'", JSON.stringify(presenterUrl));
    facade = replaceOnce(facade, "new URL('./model-worker.mjs', import.meta.url)", JSON.stringify(workerUrl));
    if (mode === 'negative') facade = replaceOnce(facade, "{type: 'classic'}", "{type: 'module'}");
    const {LocalModel} = await import(scriptUrl(facade));
    const states = [];
    model = new LocalModel(() => {}, event => states.push({...event, ready:model.ready}));
    const work = async () => {
      try {
        await model.start(new File(['synthetic'], 'fixture.litertlm'));
        const ready = model.ready;
        const worker = model.worker;
        const replies = [];
        if (mode !== 'runtime') {
          for (const role of ['Planner','Coder','Tester']) replies.push(await model.generate(role, 'fixture'));
        }
        return {ok:true, ready, sameWorker:model.worker===worker, replies, states};
      } catch (error) {return {ok:false, error:String(error.message || error), ready:model.ready, states};}
    };
    return await Promise.race([work(), new Promise(resolve => {
      timer = setTimeout(() => {model.cancel(); resolve({ok:false,error:'Browser test deadline'});}, mode==='runtime'?45000:10000);
    })]);
  } finally {
    clearTimeout(timer);
    model?.cancel();
  }
}'''

with sync_playwright() as p:
    browser = p.chromium.launch(executable_path=shutil.which('chromium'), headless=True, args=['--no-sandbox'])
    try:
        page = browser.new_page()
        # No localhost/file navigation or network required for the compatibility fixture.
        page.set_content('<!doctype html><title>Worker compatibility regression</title>')
        errors = []
        page.on('pageerror', lambda error: errors.append(str(error)))
        def run(mode):
            return page.evaluate(HARNESS, {'sources':sources, 'sdk':sdk_match.group(1), 'mode':mode})
        negative = run('negative')
        assert not negative['ok'] and not negative['ready'], negative
        assert 'importScripts' in negative.get('error',''), negative
        print('PASS negative control: module worker rejected importScripts:', negative['error'])
        result = run('classic')
        assert result['ok'] and result['ready'] and result['sameWorker'], result
        assert result['replies'] == ['synthetic reply'] * 3, result
        phases = [event['phase'] for event in result['states']]
        assert 'local' in phases and 'initializing' in phases and 'ready' in phases, phases
        assert all(not event['ready'] for event in result['states'][:phases.index('ready')]), result
        assert [event['role'] for event in result['states'] if event['phase']=='working'] == ['Planner','Coder','Tester'], result
        assert not errors, errors
        print('PASS classic worker: native importScripts, dynamic SDK import, production worker/facade, readiness gate and three role handoffs.')
        print('SDK/GPU/model are fixtures; this does NOT prove Gemma inference or Pages deployment.')
        if args.runtime:
            runtime = run('runtime')
            assert runtime['ok'] and runtime['ready'], json.dumps(runtime)
            print('PASS real pinned SDK/WASM initialization (no weights, no inference).')
    finally:
        browser.close()
