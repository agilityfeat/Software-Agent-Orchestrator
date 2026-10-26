#!/usr/bin/env python3
"""In-memory component smoke: production log UI/facade, explicit model fixtures.
No external downloads, backend, GPU inference or deployment. Requires Playwright
and Chromium. This is not a full-app or real-origin persistence test.
"""
import argparse
import json
import pathlib
import re
import shutil
import tempfile
from playwright.sync_api import sync_playwright

ROOT = pathlib.Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--screenshots', type=pathlib.Path, default=pathlib.Path(tempfile.gettempdir()) / 'agent-log-previews')
args = parser.parse_args()
args.screenshots.mkdir(parents=True, exist_ok=True)
html = (ROOT / 'demo/index.html').read_text()
html = re.sub(r'<meta http-equiv="Content-Security-Policy"[^>]+>', '', html)
html = re.sub(r'<script[^>]*>.*?</script>', '', html, flags=re.S)
html = re.sub(r'<link[^>]*>', '', html)
source = (ROOT / 'demo/run-log.mjs').read_text().replace('export class ', 'class ').replace('export function ', 'function ').replace('export const ', 'const ')
source = source.replace('import.meta.url', json.dumps('https://example.invalid/demo/run-log.mjs'))
model = (ROOT / 'demo/model.mjs').read_text().replace("import {mountModelLoader} from './model-loader.mjs';", 'const mountModelLoader=()=>()=>{};').replace('export class ', 'class ').replace('export const ', 'const ')
model = model.replace('import.meta.url', json.dumps('https://example.invalid/demo/model.mjs'))
bootstrap = '''
const workers=[];
globalThis.Worker=class {constructor(){workers.push(this)}postMessage(){}terminate(){this.terminated=true}};
globalThis.log=mountRunLog();
globalThis.model=new LocalModel(()=>{},()=>{},event=>log.model(event));
globalThis.emit=data=>workers.at(-1).onmessage({data});
globalThis.task={id:'synthetic-run',mode:'live',status:'Active',phase:'Coder',attempt:1,events:[]};
log.observe(task,true);
'''
with sync_playwright() as p:
    browser = p.chromium.launch(executable_path=shutil.which('chromium'), headless=True, args=['--no-sandbox'])
    page = browser.new_page(viewport={'width':1360, 'height':920})
    errors = []
    page.on('pageerror', lambda e: errors.append(str(e)))
    page.route('**/*', lambda route: route.abort())
    page.set_content(html)
    css = re.sub(r'@import[^;]*;', '', (ROOT / 'demo/styles.css').read_text())
    page.add_style_tag(content=css)
    page.add_style_tag(content=(ROOT / 'demo/run-log.css').read_text())
    page.add_script_tag(content=source+'\n'+model+'\n'+bootstrap)
    page.evaluate("document.getElementById('notice').hidden=false;document.getElementById('notice').textContent='Component preview: synthetic model events; no real Gemma inference.'")
    button = page.locator('#show-run-log')
    dialog = page.locator('#run-log-dialog')
    button.click()
    assert dialog.is_visible()
    assert page.locator('.run-log-list').inner_text().startswith('No activity')
    # The real facade emits logs from worker callbacks, not directly invented UI rows.
    page.evaluate('void model.start()')
    page.evaluate("emit({type:'loaded'})")
    page.evaluate("void model.generate('Coder','Synthetic app request').catch(()=>{})")
    page.evaluate("emit({type:'status',phase:'working',text:'Coder: preparing conversation and context…'})")
    page.evaluate("emit({type:'status',phase:'working',text:'Coder: generating an app patch (reasoning enabled); waiting for the complete response…'})")
    assert dialog.get_attribute('data-busy') == 'true'
    first = page.locator('.run-log-status p').inner_text()
    page.wait_for_timeout(650)
    assert page.locator('.run-log-status p').inner_text() != first
    assert page.locator('.run-log-row').count() == 5
    page.screenshot(path=str(args.screenshots/'building-logs.png'), full_page=True)
    dialog.get_by_role('button', name='Close', exact=True).click()
    assert not dialog.is_visible()
    assert page.evaluate('Boolean(model.pending)')  # Closing did not stop the model.
    assert page.evaluate("document.activeElement.id==='show-run-log'")
    output = '{"edits":[{"file":"html","find":"<th>Merchant</th>","replace":"<th>Date</th><th>Merchant</th>"}]}'
    page.evaluate('(text)=>emit({type:"generated",text})', output)
    button.click()
    assert 'not yet validated' in page.locator('.run-log-list').inner_text()
    page.locator('.run-log-row details').last.locator('summary').click()
    assert output == page.locator('.run-log-row pre').last.inner_text()
    page.evaluate("log.add('Tester','Reviewing actual browser checks');log.add('Controller','Returning a failed check to Coder.')")
    assert page.locator('.run-log-row details').last.get_attribute('open') is not None
    # Follow-latest can be paused without losing updates or disturbing an open output.
    page.locator('.run-log-dialog input[type=checkbox]').uncheck()
    page.evaluate("document.querySelector('.run-log-list').scrollTop=0;for(let i=0;i<30;i++)log.add('Browser','Synthetic check event '+i)")
    assert page.locator('.run-log-row').count() > 30
    assert page.evaluate("document.querySelector('.run-log-list').scrollTop") == 0
    page.locator('.run-log-dialog input[type=checkbox]').check()
    assert page.evaluate("document.querySelector('.run-log-list').scrollTop>0")
    # Generated code is displayed only as text.
    page.evaluate("log.add('Coder','Untrusted output','<img src=x onerror=alert(1)><script>bad()</script>')")
    assert page.locator('.run-log-list img, .run-log-list script').count() == 0
    page.evaluate("globalThis.copied='';Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:async text=>{globalThis.copied=text}}})")
    dialog.get_by_role('button',name='Copy log',exact=True).click()
    assert page.evaluate("copied.includes('[Coder]')&&copied.includes('synthetic-run')")
    assert page.locator('.run-log-feedback').inner_text() == 'Copied'
    page.keyboard.press('Escape')
    assert not dialog.is_visible()
    # Stop and late statuses cannot leave live activity behind.
    page.evaluate("void model.generate('Coder','second request').catch(()=>{});model.cancel();log.observe({...task,status:'Blocked',phase:'Cancelled'},false)")
    button.click()
    assert dialog.get_attribute('data-busy') == 'false'
    assert 'Cancelled' in page.locator('.run-log-status strong').inner_text()
    page.evaluate("log.observe({...task,id:'saved',mode:'recorded',status:'Done',phase:'Accepted locally',events:[{role:'Tester',text:'Historical checks',at:'2026-01-01T00:00:00Z'}]},false)")
    assert 'not live execution' in page.locator('.run-log-status p').inner_text()
    assert page.locator('.run-log-row').count() == 1
    page.set_viewport_size({'width':390,'height':844})
    box = dialog.bounding_box()
    assert box['width'] <= 390 and box['x'] >= 0
    assert page.evaluate("document.querySelector('.run-log-dialog').scrollWidth<=document.querySelector('.run-log-dialog').clientWidth+1")
    page.emulate_media(reduced_motion='reduce')
    page.screenshot(path=str(args.screenshots/'mobile-logs.png'),full_page=True)
    page.keyboard.press('Escape')
    assert not errors, errors
    print('PASS: modal, live elapsed status, callback-driven logs, output expansion, close/reopen without cancellation, Escape/focus return, follow pause/resume, copy, text-only output, cancellation, recorded labeling, mobile width.')
    print('Component/facade test with synthetic worker events; no GPU, real-origin app navigation, or deployed Pages validation.')
    browser.close()
