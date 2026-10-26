#!/usr/bin/env python3
"""In-memory UI smoke: real loader/facade bodies + synthetic worker events; no GPU/download.
Requires Playwright and Chromium. Screenshots are stored outside the checkout.
Run: python3 tests/model-loader-browser.py
"""
import argparse
import pathlib
import shutil
import tempfile
from playwright.sync_api import sync_playwright

ROOT = pathlib.Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--screenshots', type=pathlib.Path, default=pathlib.Path(tempfile.gettempdir()) / 'agent-loader-previews')
args = parser.parse_args()
args.screenshots.mkdir(parents=True, exist_ok=True)
ORIGIN = 'https://demo.invalid/Software-Agent-Orchestrator/'
HTML = '''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<style>
*{box-sizing:border-box}body{margin:0;padding:28px;font:14px system-ui;background:#0c1019;color:#e6e9f2}h1{font-size:22px;font-weight:500;margin:0 0 6px}p.note{color:#97a5bf;font-size:11px;margin:0 0 22px}.runtime-bar{display:flex;align-items:center;gap:14px;padding:16px;border:1px solid #30384e;background:#141b28;border-radius:12px}button,select{font:inherit;font-size:11px;border-radius:7px;padding:9px;border:1px solid #3b405c;color:#e5e8f1;background:#202738}button{cursor:pointer}button:disabled{opacity:.5;cursor:not-allowed}.primary{background:#bba5ff;color:#20153d}.runtime-copy{flex:1;min-width:140px;font-size:11px}.mode-label{font-size:11px}.file-label{font-size:10px}input{max-width:100%}[hidden]{display:none!important}@media(max-width:600px){body{padding:12px}}
</style>
<h1>Agents at Work · Local Gemma</h1><p class="note">Loader UI test · simulated worker events, not a model execution</p>
<section class="runtime-bar">
<label class="mode-label">Execution <select id="mode"><option>Live · local Gemma</option></select></label>
<div class="runtime-copy"><strong id="model-status" role="status">Local model not loaded</strong></div>
<button id="load-model" class="primary">Load Gemma · ~3 GB</button><button id="cancel" hidden>Stop</button>
</section>
<script type="module">
import {LocalModel} from './demo/model.mjs';
const workers=[];globalThis.Worker=class {constructor(){workers.push(this)}postMessage(){}terminate(){this.terminated=true}};
globalThis.emit=data=>workers.at(-1).onmessage({data});
let loading=false;
const model=new LocalModel(value=>{document.getElementById('model-status').textContent=value;document.getElementById('load-model').disabled=loading||model.ready;});
globalThis.model=model;
document.getElementById('load-model').onclick=async()=>{loading=true;try{await model.start()}catch{}finally{loading=false;document.getElementById('load-model').disabled=model.ready;}};
document.getElementById('cancel').onclick=()=>{loading=false;model.cancel();};
globalThis.generate=()=>model.generate('Coder','Synthetic task').catch(()=>{});
</script></html>'''

with sync_playwright() as p:
    browser = p.chromium.launch(executable_path=shutil.which('chromium'), headless=True, args=['--no-sandbox'])
    page = browser.new_page(viewport={'width':1060,'height':370})
    errors=[]
    page.on('pageerror',lambda e:errors.append(str(e)))
    # Use an in-memory document where browser navigation is restricted. No network bypass.
    # Strip module wiring only; exercise the unchanged production function/class bodies.
    page.set_content(HTML.split('<script type="module">')[0] + '</html>')
    page.add_style_tag(content=(ROOT / 'demo' / 'model-loader.css').read_text())
    page.evaluate("const link=document.createElement('link');link.dataset.modelLoader='';document.head.append(link)")
    presenter = (ROOT / 'demo' / 'model-loader.mjs').read_text().replace('export function ', 'function ')
    facade = (ROOT / 'demo' / 'model.mjs').read_text().replace("import {mountModelLoader} from './model-loader.mjs';", '').replace('export class ', 'class ')
    bootstrap = HTML.split('<script type="module">')[1].split('</script>')[0].replace("import {LocalModel} from './demo/model.mjs';", '')
    code = (presenter + '\n' + facade + '\n' + bootstrap).replace('import.meta.url', repr(ORIGIN + 'demo/model.mjs'))
    page.add_script_tag(content=code)
    widget=page.locator('.model-awakening')
    widget.wait_for()
    page.wait_for_function("getComputedStyle(document.querySelector('.model-awakening')).display==='grid'")
    assert widget.get_attribute('data-phase')=='idle'
    assert page.locator('.bot-badge').inner_text()=='ASLEEP'
    page.locator('#load-model').click()
    assert widget.get_attribute('data-phase')=='preparing'
    page.evaluate("emit({type:'status',phase:'downloading',text:'Downloading Gemma to this device…',loaded:900000000,total:2000000000})")
    assert page.locator('[role=progressbar]').get_attribute('aria-valuenow')=='45'
    assert page.evaluate("document.querySelector('.little-bot').getAnimations().some(a=>a.playState==='running')")
    page.wait_for_timeout(450)
    page.screenshot(path=str(args.screenshots/'downloading.png'),full_page=True)
    page.locator('.bot-motion').click()
    assert page.evaluate("document.querySelector('.model-awakening').getAnimations({subtree:true}).every(a=>a.playState==='paused')")
    page.locator('.bot-motion').click()
    page.evaluate("emit({type:'status',phase:'downloading',text:'Downloading Gemma to this device…',loaded:1000000000,total:null})")
    assert page.locator('[role=progressbar]').get_attribute('aria-valuenow') is None
    assert '%' not in page.locator('.bot-detail').inner_text()
    page.evaluate("emit({type:'status',phase:'initializing',text:'Initializing Gemma on your GPU…',source:'cache'})")
    assert page.locator('.bot-badge').inner_text()=='WAKING UP'
    assert page.evaluate('model.ready') is False
    page.screenshot(path=str(args.screenshots/'initializing.png'),full_page=True)
    page.evaluate("emit({type:'loaded'})")
    assert page.locator('.bot-badge').inner_text()=='✓ READY'
    assert page.locator('#load-model').inner_text()=='✓ Gemma loaded'
    assert page.locator('#load-model').is_disabled()
    assert page.locator('[role=progressbar]').is_hidden()
    assert page.locator('.bot-step').first.inner_text()=='01 · Cached'
    assert 'just-awoke' in widget.get_attribute('class')
    page.wait_for_timeout(800)
    page.screenshot(path=str(args.screenshots/'ready.png'),full_page=True)
    page.evaluate('void generate()')
    page.evaluate("emit({type:'status',phase:'working',text:'Coder is generating locally…',role:'Coder'})")
    assert 'Coder' in page.locator('.bot-title').inner_text()
    page.evaluate("emit({type:'generated',text:'synthetic output'})")
    assert 'just-awoke' not in widget.get_attribute('class')
    page.evaluate('model.cancel()')
    page.locator('#load-model').click()
    page.evaluate("emit({error:'Synthetic download failure <img src=x onerror=alert(1)>'})")
    assert widget.get_attribute('data-phase')=='error'
    assert page.locator('.bot-detail img').count()==0
    assert page.locator('#load-model').inner_text()=='Retry loading Gemma'
    page.locator('#load-model').click()
    page.locator('.bot-stop').click()
    assert widget.get_attribute('data-phase')=='stopped'
    page.locator('#load-model').click()
    page.evaluate("emit({type:'status',phase:'downloading',text:'Downloading',loaded:250000000,total:2000000000})")
    page.set_viewport_size({'width':390,'height':580})
    assert page.evaluate('document.documentElement.scrollWidth<=innerWidth+1')
    page.screenshot(path=str(args.screenshots/'mobile.png'),full_page=True)
    page.emulate_media(reduced_motion='reduce')
    assert page.evaluate("document.querySelector('.model-awakening').getAnimations({subtree:true}).length===0")
    assert page.locator('.bot-motion').is_hidden()
    assert page.locator('[role=progressbar]').get_attribute('aria-valuenow')=='12'
    page.locator('.bot-stop').click()
    assert not errors,errors
    print('PASS: animated assembly, real-event progress mapping, unknown length, initialization gate, ready badge, cache label, role handoff, errors, retry, stop, mobile width, reduced motion, manual pause. Module/Pages navigation is not exercised by this in-memory fixture.')
    print('UI-only test with synthetic worker events. No actual Gemma inference claimed.')
    browser.close()
