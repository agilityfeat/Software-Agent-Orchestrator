#!/usr/bin/env python3
"""Independent real-DOM full-SPA checks. NON_ACCEPTANCE for real Gemma:
only the model boundary may be synthetic; iframe/runtime/check execution is real.
External fixture requests are intercepted; never calls a live API or provider.
"""
import argparse,functools,hashlib,http.server,json,pathlib,re,shutil,subprocess,threading,tempfile,traceback
from playwright.sync_api import sync_playwright,expect
ROOT=pathlib.Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--screenshots',type=pathlib.Path,default=pathlib.Path(tempfile.gettempdir())/'page-workspace-qa');parser.add_argument('--runtime-only',action='store_true');args=parser.parse_args();args.screenshots.mkdir(parents=True,exist_ok=True)
class Handler(http.server.SimpleHTTPRequestHandler):
 def log_message(self,*args):pass
server=http.server.ThreadingHTTPServer(('127.0.0.1',0),functools.partial(Handler,directory=str(ROOT)));threading.Thread(target=server.serve_forever,daemon=True).start();BASE='http://127.0.0.1:'+str(server.server_port);URL=BASE+'/demo/'
report={'passed':False,'runtime_only':args.runtime_only,'revision':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),'source_hashes':{str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest() for p in [ROOT/'demo/app.mjs',ROOT/'demo/index.html',ROOT/'demo/styles.css',*sorted((ROOT/'demo').glob('page-*.mjs')),*sorted((ROOT/'demo/preview').glob('*'))] if p.is_file()},'label':'NON_ACCEPTANCE for real Gemma; actual production iframe and DOM checks','runtime':[],'network':[]};failures=[];report['failures']=failures
def bundle(html='<main><p id="actual">wrong</p></main>',css='',js=''):return {'html':html,'css':css,'js':js}
def assertion(selector,property='text',expected='right',operator='equals',**extra):return dict(selector=selector,property=property,operator=operator,expected=expected,**extra)
def check(assertions,steps=None,name='Actual requested behavior'):return {'name':name,'steps':steps or [],'assertions':assertions}
def run_runtime(page,name,source,checks,passing=True,error_expected=False):
 result=page.evaluate("""async({source,checks})=>{const {runPageChecks}=await import('./page-runtime.mjs');try{return await runPageChecks(source,checks,{timeout:2500});}catch(error){return {rejected:String(error.message||error)}}}""",{'source':source,'checks':checks});report['runtime'].append({'name':name,'result':result})
 passed=not result.get('rejected') and all(row.get('pass') is True for row in result.get('rows',[])) and bool(result.get('rows')) and result.get('errors')==[]
 okay=(passed if passing else not passed) and (not error_expected or result.get('errors') or result.get('rejected'))
 if not okay:failures.append({'name':name,'expected':('pass' if passing else 'fail'),'result':result})
 return result

def runtime_suite(context):
 # Use the production parent CSP on a tiny test host, with the real runtime assets.
 html=(ROOT/'demo/index.html').read_text();csp=re.search(r'http-equiv="Content-Security-Policy" content="([^"]+)"',html).group(1)
 context.route('**/__qa-runtime-host.html',lambda route:route.fulfill(content_type='text/html',body='<meta http-equiv="Content-Security-Policy" content="'+csp+'"><link rel="icon" href="data:,"><main id="host">QA host sentinel</main>'))
 def external(route):
  report['network'].append({'url':route.request.url,'method':route.request.method})
  if route.request.url=='https://fixture.example.invalid/module.mjs':route.fulfill(content_type='text/javascript',headers={'Access-Control-Allow-Origin':'*'},body='export const fixtureText="Public module loaded";')
  elif route.request.url=='https://fixture.example.invalid/data':route.fulfill(content_type='application/json',headers={'Access-Control-Allow-Origin':'*'},body='{"value":"Public fixture data"}')
  else:route.abort()
 context.route('https://**/*',external);page=context.new_page();page.goto(URL+'__qa-runtime-host.html');page.evaluate("window.__qaParentSecret='private-host-sentinel';localStorage.setItem('qa-parent-secret','private-host-sentinel');window.__qaModel={secret:'model-sentinel'}")
 todo=bundle('<main><h1>Reading list</h1><form id="add"><input id="item" aria-label="New item"><button>Add</button></form><ul id="items"></ul><p id="notice" hidden>List updated</p></main>', 'main{font-family:system-ui;background:rgb(230, 240, 250);padding:17px}button{border-radius:17px}', "document.querySelector('#add').addEventListener('submit',e=>{e.preventDefault();const input=document.querySelector('#item');const li=document.createElement('li');li.textContent=input.value;const remove=document.createElement('button');remove.textContent='Remove';remove.onclick=()=>li.remove();li.append(remove);document.querySelector('#items').append(li);input.value='';document.querySelector('#notice').hidden=false;localStorage.setItem('last-item',li.textContent);});")
 todo_checks=[check([assertion('#items li','count',1),assertion('#items','text','Read a book','includes'),assertion('#notice','visible',True),assertion('main','style','rgb(230, 240, 250)',attribute='background-color'),assertion('button','style','17px',attribute='border-radius')],[{'action':'input','selector':'#item','value':'Read a book'},{'action':'click','selector':'#add button'}],name='Non-expense app add item and arbitrary CSS'),check([assertion('#items li','count',0)],[{'action':'input','selector':'#item','value':'Own setup'},{'action':'click','selector':'#add button'},{'action':'click','selector':'#items button'}],name='Non-expense app remove item')]
 run_runtime(page,'complete replacement non-expense to-do SPA',todo,todo_checks)
 # Each top-level check starts from source with empty local/session storage; steps within one share state.
 isolated=bundle('<button id="toggle">Toggle</button><p id="actual">initial</p>',js="document.querySelector('#toggle').onclick=()=>{localStorage.setItem('touched','yes');sessionStorage.setItem('touched','yes');document.querySelector('#actual').textContent='changed'};document.body.dataset.loaded=(localStorage.getItem('touched')||'empty')+' '+(sessionStorage.getItem('touched')||'empty')")
 fresh_checks=[check([assertion('#actual',expected='changed')],[{'action':'click','selector':'#toggle'}],name='Action changes own check'),check([assertion('#actual',expected='initial'),assertion('body','attribute','empty empty',attribute='data-loaded')],name='Next check is pristine'),check([assertion('#actual',expected='changed')],name='Missing action cannot borrow prior click')]
 fresh=run_runtime(page,'fresh check DOM and local/session storage prevent borrowed action pass',isolated,fresh_checks,False)
 assert [row['pass'] for row in fresh['rows'][:-1]]==[True,True,False],fresh
 run_runtime(page,'inline HTML scripts execute before JS module',bundle('<p id="actual">wrong</p><script>document.querySelector("#actual").textContent="inline";</script>',js='document.querySelector("#actual").textContent+=" module"'),[check([assertion('#actual',expected='inline module')])])
 run_runtime(page,'public HTTPS module and API fixture obey normal browser CORS',bundle('<p id="actual"></p>',js='import {fixtureText} from "https://fixture.example.invalid/module.mjs";const data=await fetch("https://fixture.example.invalid/data").then(r=>r.json());document.querySelector("#actual").textContent=fixtureText+" "+data.value'),[check([assertion('#actual',expected='Public module loaded Public fixture data')])])
 quantifiers=[check([assertion('thead th',expected='Date')],name='Any header'),check([assertion('thead th',expected='Date',all=True)],name='Every header'),check([assertion('thead th',expected='Date',first=True)],name='First header'),check([assertion('thead th',expected='Merchant',first=True)],name='First header merchant'),check([assertion('thead th','count',3,first=True)],name='Count ignores first'),check([assertion('#none',expected='Date')],name='No matching node')]
 quantified=run_runtime(page,'explicit any all first count and missing-node semantics',bundle('<table><thead><tr><th>Merchant</th><th>Date</th><th>Amount</th></tr></thead></table>'),quantifiers,False)
 assert [row['pass'] for row in quantified['rows'][:-1]]==[True,False,False,True,True,False],quantified
 run_runtime(page,'complete document preserves root and body attributes',bundle('<!doctype html><html lang="fr" class="root-theme"><head><title>Custom app</title></head><body class="night" data-theme="dark"><p id="actual">Body theme</p></body></html>',css='body.night{background-color:rgb(10, 20, 30)}',js="document.querySelector('#actual').textContent=document.body.dataset.theme+' '+document.documentElement.lang"),[check([assertion('#actual',expected='dark fr'),assertion('body','style','rgb(10, 20, 30)',attribute='background-color')])])
 run_runtime(page,'JavaScript-generated SPA without initial HTML',bundle('',js='document.body.innerHTML="<main><h1 id=generated>Generated app</h1></main>"'),[check([assertion('#generated',expected='Generated app')])])
 run_runtime(page,'virtual app Storage follows primitive browser coercion',bundle(js="localStorage.setItem(7,12);document.querySelector('#actual').textContent=localStorage.getItem('7')"),[check([assertion('#actual',expected='12')])])
 run_runtime(page,'hash routing keeps trusted preview session',bundle(js="location.hash='reading';document.querySelector('#actual').textContent=location.hash"),[check([assertion('#actual',expected='#reading')])])
 run_runtime(page,'DOMRect prototype cannot forge zero-size visibility',bundle('<p id="actual" style="width:0;height:0;overflow:hidden">wrong</p>',js="Object.defineProperty(DOMRect.prototype,'width',{get(){return 500},configurable:true});Object.defineProperty(DOMRect.prototype,'height',{get(){return 500},configurable:true});"),[check([assertion('#actual','visible',True)])],False)
 run_runtime(page,'runtime exception cannot be a model pass',bundle(js='throw new Error("QA deliberate runtime exception")'),[check([assertion('#actual',expected='wrong')])],False,True)
 run_runtime(page,'missing DOM and invalid selector fail',bundle(),[check([assertion('#missing')]),check([assertion('[')])],False)
 run_runtime(page,'forged ordinary window messages cannot replace private results',bundle(js='for(let id=1;id<10;id++)parent.postMessage({id,result:{rows:[{pass:true}],errors:[]},type:"page-preview-boot",token:location.hash.slice(1)},"*")'),[check([assertion('#actual')])],False)
 mutations={
 'native query and text getters':"Document.prototype.querySelectorAll=()=>[document.body];Object.defineProperty(Node.prototype,'textContent',{get(){return 'right'},set(){},configurable:true});",
 'string trim/includes and native regex exec':"String.prototype.trim=()=> 'right';String.prototype.includes=()=>true;RegExp.prototype.exec=()=>['right'];",
 'native NodeList count':"Object.defineProperty(NodeList.prototype,'length',{get(){return 7},configurable:true});",
 'private MessageEvent data':"const native=Object.getOwnPropertyDescriptor(MessageEvent.prototype,'data').get;Object.defineProperty(MessageEvent.prototype,'data',{get(){const msg=native.call(this);if(msg&&msg.type==='check'){for(const c of msg.checks)for(const a of c.assertions)a.expected='wrong';}return msg},configurable:true});",
 'inherited array index setter':"Object.defineProperty(Array.prototype,'0',{configurable:true,set(value){if(value&&typeof value==='object'&&'pass'in value)value={...value,pass:true};Object.defineProperty(this,'0',{value,writable:true,configurable:true,enumerable:true})}});",
 'inherited thenable result assimilation':"Object.prototype.then=function(resolve){const out=Object.assign(Object.create(null),this);if(out.rows){for(const row of out.rows)row.pass=true;out.errors=[];}resolve(out)};",
 'captured style getter':"window.getComputedStyle=()=>({getPropertyValue:()=> 'rgb(255, 0, 0)'});CSSStyleDeclaration.prototype.getPropertyValue=()=> 'rgb(255, 0, 0)';",
 'captured input accessor':"Object.defineProperty(HTMLInputElement.prototype,'value',{get(){return 'right'},set(){},configurable:true});",
 }
 for name,js in mutations.items():
  assertions=[assertion('#actual')]
  if name=='native NodeList count':assertions=[assertion('#actual','count',7)]
  if name=='string trim/includes and native regex exec':assertions=[assertion('#actual',operator='includes'),assertion('#actual',operator='matches')]
  if name=='captured style getter':assertions=[assertion('#actual','style','rgb(255, 0, 0)',attribute='background-color')]
  html='<input id="actual" value="wrong">' if name=='captured input accessor' else '<p id="actual">wrong</p>'
  if name=='captured input accessor':assertions=[assertion('#actual','value','right')]
  run_runtime(page,name,bundle(html,js=js),[check(assertions)],False)
 isolation="""const out=[];for(const [name,read]of [['DOM',()=>parent.document.body],['Storage',()=>parent.localStorage.getItem('qa-parent-secret')],['Model',()=>parent.__qaModel.secret],['IndexedDB',()=>parent.indexedDB]]){try{read();out.push(name+':LEAK')}catch{out.push(name+':blocked')}}document.querySelector('#actual').textContent=out.join(' ');localStorage.setItem('candidate-only','isolated');"""
 run_runtime(page,'opaque parent DOM storage model and IndexedDB isolation',bundle(js=isolation),[check([assertion('#actual',expected='DOM:blocked Storage:blocked Model:blocked IndexedDB:blocked')])])
 assert page.locator('#host').inner_text()=='QA host sentinel';assert page.evaluate("localStorage.getItem('candidate-only')") is None;assert page.evaluate("localStorage.getItem('qa-parent-secret')")=='private-host-sentinel'
 # Test frames start with their own empty virtual store; accepted preview state is preserved.
 state=page.evaluate("""async source=>{const {createPagePreview,runPageChecks}=await import('./page-runtime.mjs');const div=document.createElement('div');document.body.append(div);let changes=[];const handle=await createPagePreview(div,source,{storage:{accepted:'kept'},onStorage:s=>changes.push(s),timeout:2500});const before=await handle.inspect();await runPageChecks({...source,js:"localStorage.setItem('candidate-only','poison')"},[{name:'storage',steps:[],assertions:[{selector:'#actual',property:'count',operator:'equals',expected:1}]}],{timeout:2500});const after=await handle.inspect();handle.destroy();div.remove();return {before:before.storage,after:after.storage,changes}}""",bundle())
 assert state['before']==state['after']=={'accepted':'kept'} and state['changes']==[],state;report['storage_isolation']=state
 # No real navigation request is sent: only local static frames or intercepted HTTPS fixtures.
 run_runtime(page,'iframe self-navigation invalidates trusted session',bundle(js="location.href='https://fixture.example.invalid/navigate'"),[check([assertion('#actual',expected='wrong')])],False)
 run_runtime(page,'same-origin iframe navigation invalidates session',bundle(js="location.href='./how-it-works/'"),[check([assertion('#actual',expected='wrong')])],False)
 page.close()
 return todo,todo_checks

def app_suite(context,todo,todo_checks):
 model_fixture="""export class LocalModel{
 constructor(onStatus){this.ready=false;this.onStatus=onStatus;}async start(){this.ready=true;this.onStatus('Synthetic QA model boundary — NON_ACCEPTANCE for Gemma');}cancel(){this.ready=false;}
 async generate(role,prompt){window.__qaCalls??=[];window.__qaCalls.push({role,prompt});const scenario=window.__qaScenario||'todo';if(window.__qaDelayRole===role){window.__qaModelWaiting=true;await new Promise(resolve=>window.__qaReleaseModel=resolve);}if(role==='Planner')return JSON.stringify(scenario==='delayed'?{summary:'Inspect delayed errors',steps:['Build then observe'],checks:[{name:'Delayed fixture',steps:[],assertions:[{selector:'#delayed',property:'text',operator:'equals',expected:'Ready'}]}]}:{summary:'Replace the sample with a reading list app.',steps:['Build the requested complete app.','Check forms, removal and CSS.'],checks:CHECKS});if(role==='Tester')return JSON.stringify({summary:'Review of actually executed browser checks. Synthetic model, not Gemma.',cases:[]});return JSON.stringify({files:scenario==='delayed'?{html:'<main><p id="delayed">Ready</p></main>',css:'',js:'setTimeout(()=>{throw new Error("QA delayed runtime failure")},700)'}:TODO});}
 }""".replace('CHECKS',json.dumps(todo_checks)).replace('TODO',json.dumps(todo))
 context.route('**/model.mjs',lambda route:route.fulfill(content_type='text/javascript',body=model_fixture))
 page=context.new_page();errors=[];page.on('pageerror',lambda e:errors.append(str(e)));page.goto(URL)
 def persisted():return page.evaluate("async()=>{const{load}=await import('./storage.mjs');return await load()}")
 def app_frame():
  expect(page.locator('#app-preview iframe')).to_have_count(1);return page.frame_locator('#app-preview iframe')
 def loaded():page.wait_for_function("()=>!document.querySelector('#plan').disabled")
 def done():
  page.wait_for_function("()=>['Accepted locally','Needs attention'].includes(document.querySelector('#artifact-title').textContent)");assert page.locator('#artifact-title').inner_text()=='Accepted locally',{'notice':page.locator('#notice').text_content(),'saved':persisted()};page.wait_for_function("async()=>{const{load}=await import('./storage.mjs');return(await load())?.task?.status==='Done'}")
 def reviewed():
  page.wait_for_function("()=>!document.querySelector('#accept').hidden&&!document.querySelector('#accept').disabled||document.querySelector('#artifact-title').textContent==='Needs attention'",timeout=30000)
  assert page.locator('#artifact-title').inner_text()!='Needs attention',persisted()
 def run(scenario,request):
  page.evaluate('value=>window.__qaScenario=value',scenario);page.locator('#request').fill(request);page.locator('#plan').click();page.wait_for_function("()=>!document.querySelector('#run').disabled");page.locator('#run').click()
 loaded();app_frame().locator('#expenses tr').last.wait_for();assert app_frame().locator('thead th').count()==3
 page.locator('#plan').click();page.wait_for_function("()=>!document.querySelector('#run').disabled");assert app_frame().locator('thead th').count()==3;page.locator('#run').click();reviewed();app_frame().get_by_role('columnheader',name='Date',exact=True).wait_for();assert app_frame().locator('#expenses tr').count()==6;assert '2026-10-14' in app_frame().locator('#expenses').inner_text()
 page.locator('#show-accepted').click();expect(app_frame().locator('thead th')).to_have_count(3);page.locator('#show-candidate').click();app_frame().get_by_role('columnheader',name='Date',exact=True).wait_for();page.screenshot(path=str(args.screenshots/'guided-date-candidate.png'));page.locator('#accept').click();done();assert persisted()['schema']==3
 page.reload();done();app_frame().get_by_role('columnheader',name='Date',exact=True).wait_for();page.locator('#mode').select_option('recorded');page.wait_for_function("()=>document.querySelector('#notice').textContent.includes('No successful Live')");assert page.locator('#accept').is_hidden()
 page.locator('#mode').select_option('live');page.locator('#load-model').click();page.wait_for_function("()=>!document.querySelector('#plan').disabled")
 request='Replace the expense app with a reading list where I can add and remove books, with a pale blue background and round buttons.';run('todo',request);reviewed();app_frame().get_by_role('heading',name='Reading list').wait_for();assert app_frame().locator('#expenses').count()==0
 calls=page.evaluate('window.__qaCalls');assert [c['role'] for c in calls]==['Planner','Coder','Tester'];assert request in calls[0]['prompt'];assert 'Reading list' in calls[1]['prompt'] or 'expense inbox' in calls[1]['prompt'];page.locator('[data-tab="code"]').click();assert page.locator('#artifact-body h4').all_text_contents()==['HTML','CSS','JS'];assert 'Reading list' in page.locator('#artifact-body').inner_text()
 # Candidate/test interactions cannot mutate accepted expense app or its store.
 page.wait_for_timeout(250);app_frame().locator('#item').fill('Candidate only');app_frame().locator('#add button').click();assert app_frame().locator('#items li').count()==1;assert persisted()['acceptedStorage']=={}
 page.locator('#show-accepted').click();app_frame().locator('#expenses tr').last.wait_for();assert app_frame().locator('#items').count()==0;page.locator('#show-candidate').click();app_frame().get_by_role('heading',name='Reading list').wait_for()
 page.locator('#accept').click();done();app_frame().get_by_role('heading',name='Reading list').wait_for();page.wait_for_timeout(250);app_frame().locator('#item').fill('Keep locally');app_frame().locator('#add button').click();assert app_frame().locator('#items').inner_text().startswith('Keep locally');page.wait_for_function("async()=>{const{load}=await import('./storage.mjs');return(await load()).acceptedStorage['last-item']?.startsWith('Keep locally')}")
 saved=persisted();assert saved['accepted']==todo and saved['task']['kind']=='workspace';report['accepted_bundle_hash']=saved['task']['hash'];page.screenshot(path=str(args.screenshots/'non-expense-accepted-desktop.png'));page.set_viewport_size({'width':390,'height':844});assert page.evaluate('document.documentElement.scrollWidth<=innerWidth+1');page.locator('#app-preview iframe').scroll_into_view_if_needed();app_frame().get_by_role('heading',name='Reading list').wait_for();page.wait_for_timeout(80);page.screenshot(path=str(args.screenshots/'non-expense-accepted-mobile-preview.png'));page.screenshot(path=str(args.screenshots/'non-expense-accepted-mobile.png'),full_page=True);page.set_viewport_size({'width':1512,'height':1120});page.reload();done();assert persisted()['acceptedStorage']['last-item'].startswith('Keep locally')
 page.locator('#mode').select_option('recorded');page.wait_for_function("()=>document.querySelector('#task-mode').textContent==='RECORDED'");assert page.locator('#plan').is_disabled() and page.locator('#request').is_disabled() and page.locator('#accept').is_hidden();app_frame().get_by_role('heading',name='Reading list').wait_for();assert persisted()['accepted']==todo
 assert not errors,errors;report['positive_app_pageerrors']=errors[:]
 # Fresh acceptance must block an apparently passing history entry that actually throws.
 page.evaluate(r"""async()=>{const{load,save}=await import('./storage.mjs');const{canonicalBundle,digest}=await import('./page-contract.mjs');const s=await load(),t=structuredClone(s.history.at(-1));t.candidate.js+='\nthrow new Error("QA forged saved history")';t.hash=await digest(canonicalBundle(t.candidate));t.evidence.hash=t.hash;t.evidence.rows.forEach(row=>row.pass=true);s.history=[t];s.task=null;await save(s)}""")
 page.reload();page.wait_for_function("()=>document.querySelector('#preview-status').textContent.includes('Accepted')");page.locator('#mode').select_option('live');page.locator('#mode').select_option('recorded');page.wait_for_function("()=>document.querySelector('#artifact-title').textContent==='Needs attention'");assert page.locator('#show-candidate').is_disabled() and page.locator('#accept').is_hidden();assert persisted()['accepted']==todo;app_frame().get_by_role('heading',name='Reading list').wait_for()
 # Late actual candidate preview errors cannot leave an approval button enabled.
 page.locator('#mode').select_option('live');page.locator('#load-model').click();page.wait_for_function("()=>!document.querySelector('#plan').disabled");run('delayed','Create a new page fixture and report its runtime errors.');reviewed();page.wait_for_function("()=>document.querySelector('#artifact-title').textContent==='Needs attention'",timeout=10000);assert page.locator('#accept').is_hidden() and page.locator('#show-candidate').is_disabled();assert persisted()['accepted']==todo;assert all(t['id']!=persisted()['task']['id'] for t in persisted()['history'])
 # Cancelling a real async model boundary fences late implementation and fallback history.
 page.evaluate("window.__qaDelayRole='Coder';window.__qaModelWaiting=false");run('todo','Create a fresh reading list task that will be cancelled.');page.wait_for_function('()=>window.__qaModelWaiting');page.locator('#cancel').click();page.evaluate('window.__qaReleaseModel()');page.wait_for_timeout(100);assert page.locator('#artifact-title').inner_text()=='Cancelled';assert page.locator('#show-candidate').is_disabled() and page.locator('#accept').is_hidden();assert all(t['phase']!='Cancelled' for t in persisted()['history'])
 # A previously valid source/hash cannot replay checks approved under old semantics.
 page.evaluate(r"""async original=>{const{save}=await import('./storage.mjs');const{digest}=await import('./page-contract.mjs');const s=structuredClone(original),old=structuredClone(s.task);delete old.plan.checkSemantics;old.planHash=await digest(JSON.stringify(old.plan));old.mode='live';old.status='Done';s.task=null;s.history=[old];await save(s)}""",saved)
 page.reload();page.locator('#mode').select_option('recorded');page.wait_for_function("()=>document.querySelector('#notice').textContent.includes('No successful Live')");assert page.locator('#accept').is_hidden() and page.locator('#show-candidate').is_disabled();assert persisted()['accepted']==todo
 # Previous schema is retained as an export, never accepted/executed as a full SPA.
 legacy={'schema':2,'accepted':'window.__qaLegacyExecuted=true','acceptedAppearance':{'config':{'privateLegacy':'preserved'}},'task':{'kind':'appearance','status':'Done','candidate':'not a full app'},'history':[]}
 page.evaluate("async value=>{const{save}=await import('./storage.mjs');await save(value)}",legacy);page.reload();page.wait_for_function("()=>document.querySelector('#notice').textContent.includes('previous demo workspace is preserved')");app_frame().locator('#expenses tr').last.wait_for();assert page.evaluate('window.__qaLegacyExecuted') is None;assert page.locator('#export-legacy').is_visible();assert persisted()['legacy']==legacy;assert persisted()['accepted']!=todo
 page.get_by_role('link',name='How this demo works',exact=True).click();page.wait_for_url(URL+'how-it-works/');assert page.locator('h1').inner_text()=='How this demo works';page.get_by_role('link',name='Back to the demo').click();page.wait_for_url(URL);page.get_by_role('link',name='Developer workboard',exact=True).click();page.wait_for_url(URL+'workboard/');page.wait_for_function("()=>document.querySelector('#board-connection').textContent.includes('Sample data')");page.locator('.preview-banner a').click();page.wait_for_url(URL)
 report['expected_negative_app_errors']=errors[:];report['app']={'guided_date':'actual DOM/accept/reload','generic_non_expense':'model fixture + actual add/remove/CSS/full source + accept/reload/Recorded','storage':'candidate isolated; accepted virtual state persists','forged_history':'fresh runtime failure blocks Recorded candidate','late_preview_error':'blocks acceptance and hides candidate','cancel':'late model result ignored; no fallback entry','legacy':'schema2 retained for export; baseline starts clean','old_check_semantics':'old matching-hash history excluded from Recorded','links':'help/workboard round trips','screenshots':['guided-date-candidate.png','non-expense-accepted-desktop.png','non-expense-accepted-mobile.png']};page.close()

try:
 with sync_playwright() as p:
  browser=p.chromium.launch(executable_path=shutil.which('chromium'),headless=True,args=['--no-sandbox']);context=browser.new_context(viewport={'width':1512,'height':1120});todo,todo_checks=runtime_suite(context);
  if not args.runtime_only:app_suite(context,todo,todo_checks)
  context.close();browser.close()
 report['failures']=failures;report['passed']=not failures;(args.screenshots/'runtime-result.json').write_text(json.dumps(report,indent=2)+'\n')
 if failures:raise AssertionError(json.dumps(failures,indent=2))
 print('PASS: '+str(len(report['runtime']))+' actual sandbox DOM/security cases'+(' plus full app accept/reload/Recorded/forgery/cancel/legacy workflow' if not args.runtime_only else '')+'. NON_ACCEPTANCE for Gemma.')
except Exception as error:
 report['passed']=False;report['error']=str(error);report['traceback']=traceback.format_exc();raise
finally:
 (args.screenshots/'result.json').write_text(json.dumps(report,indent=2)+'\n');server.shutdown()
