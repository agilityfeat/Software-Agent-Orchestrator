#!/usr/bin/env python3
"""Real demo recovery checks with a synthetic model boundary; not Gemma evidence.
The production Controller, app, persisted workspace and isolated preview execute.
Held model/browser promises deliberately finish or reject after Start over.
"""
import argparse
import functools
import http.server
import json
import pathlib
import re
import shutil
import threading
from playwright.sync_api import sync_playwright, expect
ROOT = pathlib.Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--browser', default=shutil.which('chromium') or shutil.which('google-chrome') or '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome')
parser.add_argument('--output', type=pathlib.Path)
args = parser.parse_args()
class Handler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass
server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), functools.partial(Handler, directory=str(ROOT)))
threading.Thread(target=server.serve_forever, daemon=True).start()
model_fixture = '''export class LocalModel{
 constructor(onStatus,_presenter,onActivity=()=>{}){this.onActivity=onActivity;this.onStatus=onStatus;this.ready=false;this.pending=null;this.epoch=0;this.identity={name:'Synthetic recovery fixture'};}
 async start(){const epoch=this.epoch;this.pending={type:'load'};this.onActivity({type:'start',operation:'load',role:'Model'});if(window.__holdLoad)await new Promise(resolve=>window.__releaseLoad=resolve);if(epoch===this.epoch){this.pending=null;this.ready=true;this.onActivity({type:'complete',operation:'load',output:''});this.onStatus('Synthetic model ready — NON_ACCEPTANCE for Gemma');}}
 cancel(){this.epoch++;this.pending=null;this.ready=false;this.onActivity({type:'stopped',message:'Synthetic model stopped'});this.onStatus('Synthetic model stopped');}
 async generate(role){const epoch=this.epoch;this.pending={type:'generate'};this.onActivity({type:'start',operation:'generate',role,inputChars:50});window.__calls??=[];window.__calls.push(role);const response=value=>{if(epoch===this.epoch)this.onActivity({type:'complete',operation:'generate',output:value});return value;};try{
  if(window.__holdRole===role)await new Promise((resolve,reject)=>{window.__held??=[];window.__held.push({role,resolve,reject});});
  if(role==='Planner')return response(JSON.stringify({summary:'Add a marker to the app.',steps:['Build the requested marker'],checks:[{name:'Marker appears',assertions:[{selector:'#marker',property:window.__badPlan?'unknown':'text',operator:'equals',expected:'Ready'}]}]}));
  if(role==='Coder')return response(JSON.stringify({files:{html:'<main><h1>Accepted recovery app</h1><p id="marker">Ready</p></main>',css:'main{font-family:system-ui;padding:24px}',js:'localStorage.setItem("accepted","saved");'}}));
  return response(JSON.stringify({summary:'Actual browser checks verify the marker.',cases:[]}));
 }finally{if(epoch===this.epoch)this.pending=null;}}
}'''
report = {'label':'NON_ACCEPTANCE for Gemma; actual app/controller/storage/iframe recovery', 'passed':False}
try:
    with sync_playwright() as p:
        browser = p.chromium.launch(executable_path=args.browser, headless=True)
        context = browser.new_context(viewport={'width':1440,'height':1100})
        context.route('https://**/*', lambda route: route.abort())
        context.route('**/model.mjs', lambda route: route.fulfill(content_type='text/javascript',body=model_fixture))
        original_runtime = (ROOT/'demo/page-runtime.mjs').read_text()
        context.route('**/page-runtime-original.mjs', lambda route: route.fulfill(content_type='text/javascript',body=original_runtime))
        wrapper = '''import{createPagePreview as create,runPageChecks as run}from'./page-runtime-original.mjs';
        export async function createPagePreview(container,bundle,options={}){window.__previewCallbacks??=[];window.__previewCallbacks.push(options.onError);return create(container,bundle,options);}
        export async function runPageChecks(...args){if(window.__holdBrowser)await new Promise(resolve=>window.__releaseBrowser=resolve);return run(...args);}'''
        context.route('**/page-runtime.mjs', lambda route: route.fulfill(content_type='text/javascript',body=wrapper))
        page = context.new_page()
        errors=[]
        page.on('pageerror', lambda error: errors.append(str(error)))
        url=f'http://127.0.0.1:{server.server_port}/demo/'
        page.goto(url)
        def stored():
            return page.evaluate("async()=>{const{load}=await import('./storage.mjs');return await load()}")
        def wait_saved_null():
            page.wait_for_function("async()=>{const{load}=await import('./storage.mjs');return(await load())?.task===null}")
        def ready():
            page.wait_for_function("()=>!document.querySelector('#plan').disabled")
        def live():
            page.locator('#mode').select_option('live')
            page.locator('#load-model').click()
            ready()
        def plan(request='Add a marker to this app.'):
            page.locator('#request').fill(request)
            page.locator('#plan').click()
            page.wait_for_function("()=>!document.querySelector('#run').disabled")
        def start_over():
            expect(page.locator('#start-over')).to_be_enabled()
            page.locator('#start-over').click()
            wait_saved_null()
            assert page.locator('#artifact-title').inner_text()=='Describe your app change'
            assert page.locator('#show-candidate').is_disabled() and page.locator('#accept').is_hidden()
        live();plan();page.locator('#run').click();page.wait_for_function("()=>!document.querySelector('#accept').hidden&&!document.querySelector('#accept').disabled")
        page.locator('#accept').click();page.wait_for_function("()=>document.querySelector('#artifact-title').textContent==='Accepted locally'")
        page.wait_for_function("async()=>{const{load}=await import('./storage.mjs');const s=await load();return s?.task?.status==='Done'&&s.acceptedStorage.accepted==='saved'}")
        baseline=stored()
        page.locator('#show-run-log').click()
        assert page.locator('#run-log-dialog').is_visible()
        assert 'browser check' in page.locator('.run-log-list').inner_text().lower()
        assert 'Accepted the tested full app locally' in page.locator('.run-log-list').inner_text()
        page.locator('#run-log-dialog').get_by_role('button',name='Close',exact=True).click()
        def unchanged():
            value=stored()
            assert value['accepted']==baseline['accepted'] and value['acceptedStorage']==baseline['acceptedStorage'], value
            assert value['history']==baseline['history'], value
            return value
        # Pending Planner can be abandoned without a stale rejection replacing the notice.
        page.evaluate("window.__holdRole='Planner'")
        page.locator('#request').fill('Plan a request that will be restarted.')
        page.locator('#plan').click();page.wait_for_function("()=>window.__held?.length===1")
        expect(page.locator('#reset')).to_be_enabled();start_over();unchanged()
        assert 'Load Gemma' in page.locator('#notice').inner_text()
        page.evaluate("window.__holdRole=null;window.__held[0].reject(Error('OLD PLANNER FAILURE'))")
        page.wait_for_timeout(80)
        assert 'OLD PLANNER' not in page.locator('#notice').inner_text()
        page.locator('#show-run-log').click()
        assert page.locator('#run-log-dialog').get_attribute('data-busy')=='false'
        assert page.locator('.run-log-status strong').inner_text()=='No active task'
        assert page.locator('.run-log-row').count()==0
        page.locator('#run-log-dialog').get_by_role('button',name='Close',exact=True).click()
        # Pending Build can restart, load again, plan anew and ignore late model/preview callbacks.
        page.locator('#load-model').click();ready();page.evaluate("window.__holdRole='Coder'");plan('Build a request that will be restarted.')
        page.locator('#run').click();page.wait_for_function("()=>window.__held?.length===2")
        page.evaluate("()=>{window.__oldPreviewError=window.__previewCallbacks.at(-1)}")
        start_over();unchanged()
        page.locator('#load-model').click();ready();page.evaluate("window.__holdRole=null")
        plan('Fresh plan after restarting Build.')
        fresh_id=stored()['task']['id']
        page.evaluate("window.__held[1].resolve();window.__oldPreviewError?.('OLD PREVIEW FAILURE');window.postMessage({type:'page-preview-boot',token:'old',result:{rows:[{pass:true}]}},'*')")
        page.wait_for_timeout(100)
        assert stored()['task']['id']==fresh_id and stored()['task']['status']=='Ready'
        assert 'OLD' not in page.locator('#notice').inner_text()
        page.locator('#show-run-log').click()
        assert page.locator('#run-log-dialog').get_attribute('data-busy')=='false'
        assert 'OLD' not in page.locator('.run-log-list').inner_text()
        assert 'Add a marker to the app.' in page.locator('.run-log-list').inner_text()
        page.locator('#run-log-dialog').get_by_role('button',name='Close',exact=True).click()
        # Planner format failures expose the real reason; idle loaded model remains ready after recovery.
        start_over();page.evaluate('window.__badPlan=true');page.locator('#request').fill('A request with a deliberately invalid model plan.');page.locator('#plan').click()
        page.wait_for_function("()=>document.querySelector('#artifact-title').textContent==='Needs attention'")
        assert 'Unknown browser assertion' in page.locator('#artifact-body').inner_text()
        assert 'planner is reviewing' not in page.locator('#artifact-body').inner_text()
        start_over();unchanged();assert page.locator('#load-model').is_disabled();ready()
        page.evaluate('window.__badPlan=false');plan('A valid request after the blocked plan.')
        # Leaving an in-flight Recorded verification cannot post an obsolete error.
        page.evaluate('window.__holdBrowser=true');page.locator('#mode').select_option('recorded');page.wait_for_function("()=>typeof window.__releaseBrowser==='function'")
        page.locator('#mode').select_option('guided');page.locator('#request').fill('A new request after leaving Recorded.')
        page.evaluate('window.__holdBrowser=false;window.__releaseBrowser()');page.wait_for_timeout(100)
        assert page.locator('#notice').is_hidden() and page.locator('#artifact-title').inner_text()=='Describe your app change'
        assert page.locator('#request').is_enabled()
        page.locator('#mode').select_option('live');plan('A valid request before the reload check.')
        # Reloading an active Build produces an actionable interrupted state.
        page.evaluate("window.__holdRole='Coder'");page.locator('#run').click();page.wait_for_function("()=>window.__held?.length===3")
        page.wait_for_function("async()=>{const{load}=await import('./storage.mjs');return(await load())?.task?.status==='Active'}")
        page.reload();page.wait_for_function("()=>document.querySelector('#artifact-title').textContent==='Interrupted by reload'")
        assert 'interrupted' in page.locator('#artifact-body').inner_text();start_over();unchanged()
        # A previously completed workspace can be restarted during fresh restore verification.
        page.evaluate("async value=>{const{save}=await import('./storage.mjs');await save(value)}",baseline)
        page.add_init_script('window.__holdBrowser=true')
        page.reload();page.wait_for_function('()=>typeof window.__releaseBrowser===\'function\'')
        start_over();unchanged();page.evaluate("window.__holdBrowser=false;window.__releaseBrowser()")
        page.wait_for_timeout(100)
        assert page.locator('#artifact-title').inner_text()=='Describe your app change'
        assert 'cancelled' not in page.locator('#notice').inner_text().lower()
        # Active model loading has the same safe recovery and late-finally fence.
        page.evaluate('window.__holdLoad=true');page.locator('#load-model').click();page.wait_for_function("()=>typeof window.__releaseLoad==='function'")
        start_over();page.evaluate('window.__holdLoad=false;window.__releaseLoad()');page.wait_for_timeout(80)
        assert page.locator('#load-model').is_enabled() and page.locator('#request').is_enabled();unchanged()
        page.locator('#load-model').click();ready();plan('Reset while a new Build is active.')
        page.evaluate("window.__holdRole='Coder'");page.locator('#run').click();page.wait_for_function("()=>window.__held?.length===1")
        page.once('dialog', lambda dialog: dialog.accept());page.locator('#reset').click();wait_saved_null()
        reset=stored();initial=page.evaluate("async()=>{const{INITIAL_BUNDLE}=await import('./page-baseline.mjs');return INITIAL_BUNDLE}")
        assert reset['accepted']==initial and reset['acceptedStorage']=={} and reset['history']==[]
        page.evaluate('window.__held[0].reject(Error("OLD RESET BUILD"))');page.wait_for_timeout(80)
        assert 'OLD RESET' not in page.locator('#notice').inner_text() and page.locator('#request').is_enabled()
        assert not errors, errors
        report.update(passed=True,baseline_history=len(baseline['history']),accepted_preserved=True,scenarios=['active Planner restart + late rejection','active Build restart + late model/preview/window messages','new valid plan after restart','blocked actual plan error + idle model preservation','leave Recorded while verification is pending','reload interrupted Build','restart during saved workspace verification','model loading restart + late completion','active Reset demo + late Build rejection','RunLog modal completion + restart reset + old-callback fences'],pageerrors=errors)
        if args.output:
            args.output.parent.mkdir(parents=True,exist_ok=True)
            args.output.write_text(json.dumps(report,indent=2)+'\n')
        print('PASS: active/blocked/reload/load/restore recovery, preserved accepted app/history, active Reset, fresh planning and stale callback fences')
        browser.close()
finally:
    server.shutdown()
