#!/usr/bin/env python3
"""Independent real-browser checks of the official read-only sample workboard.
No backend, provider calls, authentication, model inference or request interception
is used to supply application data: the production static preview adapter does it.
"""
import argparse,functools,hashlib,http.server,json,pathlib,shutil,subprocess,tempfile,threading
from playwright.sync_api import sync_playwright

ROOT=pathlib.Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--screenshots',type=pathlib.Path,default=pathlib.Path(tempfile.gettempdir())/'workboard-preview-qa')
args=parser.parse_args();args.screenshots.mkdir(parents=True,exist_ok=True)
class Handler(http.server.SimpleHTTPRequestHandler):
    def log_message(self,*args):pass
server=http.server.ThreadingHTTPServer(('127.0.0.1',0),functools.partial(Handler,directory=str(ROOT)))
threading.Thread(target=server.serve_forever,daemon=True).start()
base='http://127.0.0.1:'+str(server.server_port)
url=base+'/demo/workboard/'
report={'url':url,'scope':'Actual generated official UI and production fictional adapter; no live runtime',
        'revision':subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
        'source_hashes':{str(path.relative_to(ROOT)):hashlib.sha256(path.read_bytes()).hexdigest() for path in sorted((ROOT/'demo/workboard').glob('*')) if path.is_file()}}
def readonly(page):
    assert page.locator('[data-command]:not([data-capability="read"])').evaluate_all('nodes=>nodes.every(node=>node.disabled)')
    assert page.locator('.task-drag-handle').evaluate_all('nodes=>nodes.every(node=>!node.draggable)')
def detail(page):
    page.wait_for_function("() => document.querySelector('#task-detail').open && document.querySelector('#detail-title')")
    assert page.locator('#detail-title').inner_text()
    readonly(page)
    page.locator('#close-detail').click()
def nav(page,name):
    page.locator('[data-view="'+name+'"]').click()
    assert page.locator('#'+name+'-view').is_visible()

try:
    report['generator']=subprocess.check_output(['node','scripts/generate-workboard-preview.mjs','--check'],cwd=ROOT,text=True).strip()
    for filename in ['styles.css','workboard-model.js']:
        expected=(ROOT/'public'/filename).read_bytes();actual=(ROOT/'demo/workboard'/filename).read_bytes()
        assert expected==actual;report['parity_'+filename]=hashlib.sha256(actual).hexdigest()
    with sync_playwright() as p:
        browser=p.chromium.launch(executable_path=shutil.which('chromium'),headless=True,args=['--no-sandbox'])
        context=browser.new_context(viewport={'width':1512,'height':1120},device_scale_factor=1)
        context.add_init_script("""window.__qaTransport={fetch:[],events:[],sockets:[]};
window.fetch=(...args)=>{window.__qaTransport.fetch.push(String(args[0]));return Promise.reject(Error('Unexpected fetch in static preview'));};
window.EventSource=class{constructor(url){window.__qaTransport.events.push(String(url));throw Error('Unexpected EventSource');}};
window.WebSocket=class{constructor(url){window.__qaTransport.sockets.push(String(url));throw Error('Unexpected WebSocket');}};""")
        page=context.new_page();requests=[];errors=[];console=[];failed=[];responses=[]
        page.on('request',lambda r:requests.append({'url':r.url,'method':r.method,'type':r.resource_type}))
        page.on('pageerror',lambda e:errors.append(str(e)))
        page.on('console',lambda m:console.append({'text':m.text,'location':m.location,'page':page.url}) if m.type=='error' else None)
        page.on('requestfailed',lambda r:failed.append({'url':r.url,'error':r.failure}))
        page.on('response',lambda r:responses.append({'url':r.url,'status':r.status}))
        page.goto(url);page.wait_for_function("() => document.getElementById('board-connection').textContent.includes('Sample data')")
        assert page.locator('.preview-banner').is_visible()
        assert 'Read-only preview' in page.locator('.preview-banner').inner_text()
        assert not page.locator('#access-panel').is_visible()
        assert not page.locator('#operator-key').is_visible()
        assert not page.locator('#sign-out').is_visible()
        assert page.locator('#new-task').is_disabled()
        assert page.locator('#task-board .task-card').count()>=10
        stages=page.locator('#task-board .board-column').evaluate_all('nodes=>nodes.map(node=>({stage:node.dataset.stage,count:node.querySelectorAll(".task-card").length}))')
        assert {item['stage'] for item in stages}=={'backlog','ready','active','review','done','cancelled'}
        assert all(item['count']>=1 for item in stages),stages
        report['stages']=stages;readonly(page)
        page.screenshot(path=str(args.screenshots/'work-desktop.png'),full_page=True)
        page.locator('.task-card-open').first.click();detail(page)
        page.locator('#board-filter-menu').evaluate('node=>node.open=true')
        page.locator('#project-filter').select_option(label='Field notes sample')
        expected_project=page.evaluate("async()=>{const {previewRequest}=await import(new URL('preview-data.mjs',location.href));return (await previewRequest('/snapshot')).tasks.filter(task=>task.project==='Field notes sample'&&!task.archived_at).length;}")
        assert page.locator('.task-card-open').count()==expected_project
        assert expected_project>=1 and all('Field notes sample' in value for value in page.locator('.task-card-open').all_text_contents())
        page.locator('#reset-filters').click()
        page.locator('#task-search').fill('receipt')
        assert page.locator('.task-card-open').count()==1
        assert 'receipt' in page.locator('.task-card-open').inner_text().lower()
        page.locator('#task-search').fill('')
        page.locator('#archive-filter').select_option('archive')
        assert page.locator('.task-card-open').count()==1
        assert 'Archive the old welcome copy' in page.locator('.task-card-open').inner_text()
        page.locator('#reset-filters').click()
        nav(page,'attention');assert page.locator('.attention-card').count()>=2
        page.screenshot(path=str(args.screenshots/'attention.png'),full_page=True)
        page.locator('.attention-card').first.click();detail(page)
        nav(page,'system');assert page.locator('#system-agents article').count()==3
        assert page.locator('#edit-agents').is_disabled()
        assert 'no agents are running' in page.locator('#system-status').inner_text()
        assert page.locator('#system-activity button[data-capability="read"]').count()>=1
        page.screenshot(path=str(args.screenshots/'system.png'),full_page=True)
        page.locator('#system-activity button[data-capability="read"]').first.click();detail(page)
        nav(page,'main')
        assert page.locator('#main-goal-form input,#main-goal-form textarea,#main-generate').evaluate_all('nodes=>nodes.every(node=>node.disabled)')
        assert page.locator('#main-plans button[data-capability="read"]').count()==3
        page.screenshot(path=str(args.screenshots/'main.png'),full_page=True)
        page.locator('#main-plans button[data-capability="read"]').first.click();detail(page)
        adapter=page.evaluate("""async()=>{
const {previewRequest}=await import(new URL('preview-data.mjs',location.href));
const original=await previewRequest('/snapshot'),before=JSON.stringify(original),rejections=[];
for(const method of ['POST','PUT','PATCH','DELETE','HEAD','OPTIONS','post']){try{await previewRequest('/snapshot',{method});rejections.push({method,rejected:false});}catch(error){rejections.push({method,rejected:true,status:error.status});}}
for(const path of ['/unknown','/snapshot?private=1','https://example.invalid/api','/tasks/not-real']){try{await previewRequest(path);rejections.push({path,rejected:false});}catch(error){rejections.push({path,rejected:true,status:error.status});}}
try{await previewRequest('/snapshot',{body:{change:true}});rejections.push({bodyRejected:false});}catch(error){rejections.push({bodyRejected:error.status===403});}
original.tasks[0].title='MUTATED COPY';original.capabilities.write=true;
const after=await previewRequest('/snapshot'),session=await previewRequest('/session'),item=await previewRequest('/tasks/'+after.tasks[0].id);
return {rejections,unchanged:JSON.stringify(after)===before,taskCount:after.tasks.length,ids:after.tasks.map(task=>task.id),capabilities:after.capabilities,sessionIdentity:session.identity,detailRead:!!item.task.title,serialized:JSON.stringify(after)};
}""")
        assert all(item.get('rejected',item.get('bodyRejected',False)) for item in adapter['rejections']),adapter['rejections']
        assert adapter['unchanged'] and adapter['detailRead'] and adapter['taskCount']==12
        assert all(identifier.startswith('sample-') for identifier in adapter['ids'])
        assert adapter['capabilities']['read'] and not any(value for key,value in adapter['capabilities'].items() if key!='read')
        assert not any(value in adapter['serialized'].lower() for value in ['/users/','/home/','alberto','agonza1','ghp_','github_pat_'])
        adapter.pop('serialized');report['adapter']=adapter
        # Even a programmatic form submission cannot bypass the production adapter.
        page.locator('#main-goal-form').evaluate("node=>node.dispatchEvent(new Event('submit',{bubbles:true,cancelable:true}))")
        page.wait_for_function("() => document.getElementById('main-error').textContent.length>0")
        assert not page.locator('#access-panel').is_visible()
        nav(page,'work');page.set_viewport_size({'width':390,'height':844})
        assert page.evaluate('document.documentElement.scrollWidth<=innerWidth+1')
        page.locator('#board-stages button[data-stage="review"]').click()
        assert page.locator('#board-stages button[data-stage="review"]').get_attribute('aria-pressed')=='true'
        page.screenshot(path=str(args.screenshots/'work-mobile.png'),full_page=False)
        assert page.locator('.preview-banner a').get_attribute('href')=='../'
        report['initial_transport']=page.evaluate('window.__qaTransport')
        assert not any(report['initial_transport'].values()),report['initial_transport']
        assert not errors and not console and not failed,(errors,console,failed)
        page.locator('.preview-banner a').click();page.wait_for_url(base+'/demo/')
        assert page.get_by_role('link',name='Developer workboard',exact=True).get_attribute('href')=='./workboard/'
        page.get_by_role('link',name='Developer workboard',exact=True).click();page.wait_for_url(url)
        page.wait_for_function("() => document.getElementById('board-connection').textContent.includes('Sample data')")
        report['transport']=page.evaluate('window.__qaTransport')
        assert not any(report['transport'].values()),report['transport']
        report['console_diagnostics']=console
        assert not errors and not console and not failed,(errors,console,failed,[response for response in responses if response['status']>=400])
        assert all(request['method']=='GET' and '/api/' not in request['url'] and request['type']!='eventsource' for request in requests),requests
        assert all(response['status']<400 for response in responses),responses
        assert all(request['url'].startswith(base+'/demo/') for request in requests),requests
        report.update({'requests':requests,'pageerrors':errors,'console_errors':console,'request_failures':failed,'passed':True})
        (args.screenshots/'result.json').write_text(json.dumps(report,indent=2)+'\n')
        print('PASS: official generated UI, six populated stages, detail/search/project/archive, Attention/System/Main, disabled mutations, GET-only frozen adapter, no auth/API/stream/socket/write traffic, copied asset parity, mobile width, nested relative links.')
        browser.close()
finally:
    server.shutdown()
