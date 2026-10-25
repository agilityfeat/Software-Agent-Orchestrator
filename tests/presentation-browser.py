#!/usr/bin/env python3
"""Independent browser regressions: actual app/QuickJS, synthetic model boundary.
NON_ACCEPTANCE for Gemma: this fixture never downloads weights or proves inference.
Run: python3 tests/presentation-browser.py --screenshots /tmp/presentation-qa
"""
import argparse,functools,http.server,json,pathlib,shutil,threading,tempfile
from playwright.sync_api import sync_playwright

ROOT=pathlib.Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--screenshots',type=pathlib.Path,default=pathlib.Path(tempfile.gettempdir())/'presentation-qa')
args=parser.parse_args();args.screenshots.mkdir(parents=True,exist_ok=True)
class Handler(http.server.SimpleHTTPRequestHandler):
    def log_message(self,*args):pass
server=http.server.ThreadingHTTPServer(('127.0.0.1',0),functools.partial(Handler,directory=str(ROOT)))
threading.Thread(target=server.serve_forever,daemon=True).start()
URL='http://127.0.0.1:'+str(server.server_port)+'/demo/'

MODEL_FIXTURE=r"""
export class LocalModel {
 constructor(onStatus){this.ready=false;this.onStatus=onStatus;}
 async start(){this.ready=true;this.onStatus('QA model adapter: no Gemma inference');}
 cancel(){this.ready=false;}
 async generate(role,prompt){
  window.__qaCalls??=[];window.__qaCalls.push({role,prompt});
  const scenario=window.__qaScenario||'receipt';
  if(role==='Planner')return JSON.stringify(scenario==='policy'?{scope:'policy',summary:'Review explicit policy.',steps:['Implement and check.']}:{scope:'appearance',summary:'Review the requested appearance.',steps:['Change the approved surface.','Check readability and preserved expenses.'],goals:[{target:scenario==='receipt'?'receiptDialog':scenario==='cards'||scenario==='contrast'?'cards':'headings',intent:scenario==='receipt'?'match-app':'adjust'}]});
  if(role==='Tester'){if(scenario==='delayPreview')window.__qaDelayNextDigest=true;return JSON.stringify({summary:'Synthetic model review of executed host evidence.',cases:[]});}
  if(scenario==='policy'){
   const core=await import('./core.mjs');return core.fixtureSource(core.policy('500'));
  }
  const m=await import('./presentation.mjs'),base=m.captureAppearance();
  let changes;
  if(scenario==='receipt')changes=['receiptDialog','receiptButton','dialogButton'].flatMap(target=>Object.entries(m.matchProfile(target,base)).map(([property,value])=>({target,property,value:typeof value==='number'?String(value):value})));
  else if(scenario==='cards')changes=[{target:'cards',property:'borderRadius',value:20}];
  else if(scenario==='contrast')changes=[{target:'cards',property:'background',value:'#8190a4'},{target:'cards',property:'color',value:'#000000'}];
  else changes=[{target:'headings',property:'fontSize',value:24}];
  return JSON.stringify({changes});
 }
}
"""
def install(context):
    context.route('**/model.mjs',lambda route:route.fulfill(content_type='text/javascript',body=MODEL_FIXTURE))
    context.route('https://**/*',lambda route:route.abort())
    context.add_init_script("""window.__qaScenario='receipt';window.__qaCalls=[];window.__qaDigestWaiting=false;
const original=crypto.subtle.digest.bind(crypto.subtle);
crypto.subtle.digest=async(...args)=>{if(window.__qaDelayNextDigest){window.__qaDelayNextDigest=false;window.__qaDigestWaiting=true;await new Promise(resolve=>window.__qaReleaseDigest=resolve);}return original(...args);};""")
def loaded(page):
    page.goto(URL);page.wait_for_function("() => document.querySelectorAll('#expenses tr').length===6")
def ready_model(page):
    if not page.locator('#load-model').is_disabled():page.locator('#load-model').click()
    page.locator('#mode').select_option('live')
def run(page,scenario,request):
    page.evaluate('(value)=>window.__qaScenario=value',scenario)
    page.locator('#request').fill(request);page.locator('#plan').click()
    page.wait_for_function("() => !document.getElementById('run').disabled")
    page.locator('#run').click()
def review(page):
    page.wait_for_function("() => (!document.getElementById('accept').hidden && !document.getElementById('accept').disabled) || document.getElementById('artifact-title').textContent==='Needs attention'",timeout=30000)
    if page.locator('#accept').is_hidden():
        saved=state(page);(args.screenshots/'blocked-state.json').write_text(json.dumps(saved,indent=2)+'\n')
        raise AssertionError({'notice':page.locator('#notice').inner_text(),'status':saved['task']['status'],'kind':saved['task'].get('kind'),'evidence':saved['task'].get('evidence',{}).get('rows',[])})
def accept(page):
    page.locator('#accept').click();page.wait_for_function("() => document.getElementById('artifact-title').textContent==='Accepted locally'");page.wait_for_timeout(30)
def state(page):
    return page.evaluate("async()=>{const {load}=await import(new URL('storage.mjs',location.href));return await load();}")
def color(page,selector,property):
    return page.locator(selector).first.evaluate('(node,property)=>getComputedStyle(node)[property]',property)

try:
 with sync_playwright() as p:
  browser=p.chromium.launch(executable_path=shutil.which('chromium'),headless=True,args=['--no-sandbox'])
  context=browser.new_context(viewport={'width':1512,'height':1120},device_scale_factor=1);install(context)
  page=context.new_page();errors=[];page.on('pageerror',lambda error:errors.append(str(error)))
  loaded(page);baseline=page.locator('#expenses .badge').all_text_contents();receipt_text=None
  page.locator('[data-receipt-trigger]').click();receipt_text=page.locator('#receipt-copy').inner_text()
  before_dialog=color(page,'#receipt-dialog','backgroundColor');page.screenshot(path=str(args.screenshots/'receipt-before.png'))
  page.locator('#receipt-dialog button').click();ready_model(page)
  request='Make view receipt match the rest of the look and feel.'
  run(page,'receipt',request);review(page)
  assert color(page,'#receipt-dialog','backgroundColor')=='rgb(255, 255, 255)'
  assert color(page,'[data-receipt-trigger]','fontSize')=='12px'
  assert page.locator('#expenses .badge').all_text_contents()==baseline
  page.locator('[data-receipt-trigger]').click()
  assert page.locator('#receipt-copy').inner_text()==receipt_text
  page.screenshot(path=str(args.screenshots/'receipt-candidate.png'));page.locator('#receipt-dialog button').click()
  page.locator('#show-accepted').click();assert color(page,'#receipt-dialog','backgroundColor')==before_dialog
  page.locator('#show-candidate').click();accept(page)
  saved=state(page);assert saved['schema']==2 and saved['task']['kind']=='appearance'
  assert saved['accepted']==saved['task']['policySnapshot']['source']
  assert saved['task']['versions'][-1]['rawResponse']
  page.reload();page.wait_for_function("() => document.getElementById('artifact-title').textContent==='Accepted locally'")
  assert color(page,'#receipt-dialog','backgroundColor')=='rgb(255, 255, 255)'
  page.locator('#mode').select_option('recorded')
  page.wait_for_function("() => document.getElementById('task-mode').textContent==='RECORDED'")
  assert page.locator('#accept').is_hidden() and page.locator('#plan').is_disabled()
  assert page.locator('#request').is_disabled()
  ready_model(page);second='Give the summary cards rounder corners.'
  run(page,'cards',second);review(page)
  calls=page.evaluate('window.__qaCalls');assert any(call['role']=='Planner' and second in call['prompt'] for call in calls)
  assert [call['role'] for call in calls[-3:]]==['Planner','Coder','Tester']
  assert color(page,'.stats>div','borderRadius')=='20px'
  assert color(page,'#receipt-dialog','backgroundColor')=='rgb(255, 255, 255)'
  accept(page);assert state(page)['accepted']==saved['accepted']
  page.reload();page.wait_for_function("() => document.getElementById('artifact-title').textContent==='Accepted locally'")
  assert color(page,'.stats>div','borderRadius')=='20px'
  ready_model(page)
  run(page,'policy','Require manager approval above $500. Preserve already paid expenses.');review(page);accept(page)
  assert page.locator('#needs-count').inner_text()=='2'
  assert page.locator('#expenses tr').filter(has_text='Studio Supply').locator('.badge').inner_text()=='Eligible'
  assert page.locator('#expenses tr').filter(has_text='Cloud Hosting').locator('.badge').inner_text()=='Paid'
  assert color(page,'#receipt-dialog','backgroundColor')=='rgb(255, 255, 255)'
  accepted_policy=state(page)['accepted']
  run(page,'contrast','Change the summary card colors.');page.wait_for_function("() => document.getElementById('notice').textContent.includes('contrast became unreadable')")
  assert page.locator('#accept').is_hidden() and page.locator('#show-candidate').is_disabled()
  assert color(page,'.stats>div','backgroundColor')=='rgb(255, 255, 255)'
  assert state(page)['accepted']==accepted_policy
  # A saved genuine earlier Review is stale after later accepted appearance changes.
  page.evaluate("""async()=>{const {load,save}=await import(new URL('storage.mjs',location.href));const s=await load();const task=structuredClone(s.history.find(task=>task.kind==='appearance'));const {digest,EXPENSES}=await import(new URL('core.mjs',location.href));const {execute}=await import(new URL('sandbox.mjs',location.href));task.policySnapshot={source:s.accepted,hash:await digest(s.accepted),outputs:await execute(s.accepted,EXPENSES)};task.status='Review';task.phase='Human approval';s.task=task;await save(s);}""")
  page.reload();page.wait_for_function("() => document.getElementById('notice').textContent.includes('accepted appearance changed')")
  assert page.locator('#accept').is_hidden()
  assert color(page,'.stats>div','borderRadius')=='20px'
  # Same-color regression is blocked across DOM reconstruction, not only token checks.
  assert not errors,errors
  print('PASS: actual QuickJS + synthetic model boundary: receipt preview/accept/reload/Recorded, distinct generic target, preserved policy/appearance, descendant contrast rejection, stale Review rejection.')
  context.close()
  # Control a real browser digest boundary to cancel after host checks but before preview.
  context=browser.new_context();install(context);page=context.new_page();loaded(page);ready_model(page)
  run(page,'delayPreview','Make the app heading smaller.')
  page.wait_for_function("() => window.__qaDigestWaiting")
  page.locator('#cancel').click();page.evaluate('window.__qaReleaseDigest()')
  page.wait_for_timeout(100)
  assert page.locator('#artifact-title').inner_text()=='Cancelled'
  assert page.locator('#show-candidate').is_disabled() and page.locator('#accept').is_hidden()
  cancelled=state(page);assert all(task['status'] in ['Review','Done'] for task in cancelled.get('history',[])),cancelled.get('history')
  print('PASS: late appearance preview cannot revive cancellation or create a Recorded fallback.')
  browser.close()
finally:
 server.shutdown()
