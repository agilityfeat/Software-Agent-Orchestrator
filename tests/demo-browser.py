#!/usr/bin/env python3
"""Browser smoke test. Default uses bundled real QuickJS; --offline-ui uses a test-only Node VM adapter.
Requires: python -m pip install playwright; python -m playwright install chromium.
Never loads Gemma or calls a paid provider. Screenshots go outside the checkout.
"""
import argparse
import functools
import http.server
import json
import pathlib
import shutil
import subprocess
import tempfile
import threading
from playwright.sync_api import sync_playwright

ROOT = pathlib.Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--offline-ui', action='store_true')
parser.add_argument('--screenshots', type=pathlib.Path, default=pathlib.Path(tempfile.gettempdir()) / 'agent-demo-screenshots')
args = parser.parse_args()
args.screenshots.mkdir(parents=True, exist_ok=True)

class Handler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass

server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), functools.partial(Handler, directory=str(ROOT)))
threading.Thread(target=server.serve_forever, daemon=True).start()
url = f'http://127.0.0.1:{server.server_port}/demo/'

def test_execute(source, inputs):
    # Only synthetic app fixtures are passed here. This is NOT a security sandbox.
    program = """const vm=require('node:vm');let s='';process.stdin.on('data',c=>s+=c);process.stdin.on('end',()=>{const {source,inputs}=JSON.parse(s);const result=inputs.map(input=>JSON.parse(vm.runInNewContext(source+';JSON.stringify(evaluateExpense('+JSON.stringify(input)+'))',Object.create(null),{timeout:100})));console.log(JSON.stringify(result));});"""
    result = subprocess.run(['node', '-e', program], input=json.dumps({'source': source, 'inputs': inputs}), text=True, capture_output=True, timeout=10, check=True)
    return json.loads(result.stdout)

try:
    with sync_playwright() as p:
        browser = p.chromium.launch(executable_path=shutil.which('chromium'), headless=True, args=['--no-sandbox'])
        context = browser.new_context(viewport={'width': 1512, 'height': 1120}, device_scale_factor=1)
        page = context.new_page()
        errors = []
        external_requests = []
        def block_external(route):
            external_requests.append(route.request.url)
            route.abort()
        # Candidate execution must work from a project subpath with no CDN access.
        context.route('https://**/*', block_external)
        page.on('pageerror', lambda error: errors.append(str(error)))
        if args.offline_ui:
            page.expose_function('__testExecute', test_execute)
            page.route('**/sandbox.mjs', lambda route: route.fulfill(content_type='application/javascript', body="export const execute=(source,inputs)=>globalThis.__testExecute(source,inputs);export const cancelAll=()=>{};"))
        page.goto(url)
        page.wait_for_function("() => document.getElementById('expenses').children.length===6")
        assert page.locator('#needs-count').inner_text() == '0'
        if not args.offline_ui:
            isolation = page.evaluate('''async () => {
              const {execute} = await import(new URL('sandbox.mjs', location.href));
              const outputs = await execute(`function evaluateExpense() {
                return {status:'eligible',canApprove:true,reason:JSON.stringify([
                  typeof document,typeof window,typeof self,typeof fetch,
                  typeof process,typeof require,typeof postMessage])};
              }`, [{}]);
              let loopError;
              try {await execute('function evaluateExpense() {while(true){}}', [{}]);}
              catch (error) {loopError=String(error.message || error);}
              const recovered=await execute("function evaluateExpense() {return {status:'paid'};}", [{}]);
              return {globals:JSON.parse(outputs[0].reason),loopError,recovered};
            }''')
            assert isolation['globals'] == ['undefined'] * 7, isolation
            assert 'execution budget' in isolation.get('loopError', ''), isolation
            assert isolation['recovered'] == [{'status':'paid'}], isolation
            print('PASS: real QuickJS hides host APIs, interrupts infinite loops and recovers in a fresh sandbox.')
        page.locator('#plan').click()
        page.wait_for_function("() => !document.getElementById('run').disabled")
        assert page.locator('#artifact-title').inner_text() == 'Awaiting authorization'
        page.locator('#run').click()
        page.wait_for_function("() => !document.getElementById('accept').hidden && !document.getElementById('accept').disabled", timeout=60000)
        assert page.locator('#test-count').inner_text() == '9/9'
        assert page.locator('#attempt').inner_text() == 'Attempt 2/3'
        assert page.locator('#needs-count').inner_text() == '2'
        for merchant in ['Studio Supply', 'Design Tools']:
            assert page.locator('#expenses tr').filter(has_text=merchant).locator('.badge').inner_text() == 'Eligible'
        assert page.locator('#expenses tr').filter(has_text='Cloud Hosting').locator('.badge').inner_text() == 'Paid'
        page.screenshot(path=str(args.screenshots / 'side-by-side.png'), full_page=True)
        page.locator('#layout').click()
        assert 'stacked' in page.locator('#workspace').get_attribute('class')
        page.screenshot(path=str(args.screenshots / 'top-bottom.png'), full_page=True)
        page.locator('#layout').click()
        page.locator('#show-accepted').click()
        assert page.locator('#needs-count').inner_text() == '0'
        page.locator('#show-candidate').click()
        page.locator('#accept').click()
        page.wait_for_function("() => document.getElementById('artifact-title').textContent==='Accepted locally'")
        assert page.locator('#needs-count').inner_text() == '2'
        page.wait_for_timeout(300)
        page.reload()
        page.wait_for_function("() => document.getElementById('needs-count').textContent==='2'")
        for merchant in ['Studio Supply', 'Design Tools']:
            assert page.locator('#expenses tr').filter(has_text=merchant).locator('.badge').inner_text() == 'Eligible'
        assert page.locator('#expenses tr').filter(has_text='Cloud Hosting').locator('.badge').inner_text() == 'Paid'
        page.locator('#search').fill('Atlas')
        assert page.locator('#expenses tr').count() == 1
        page.locator('#search').fill('')
        page.locator('#filter').select_option('attention')
        assert page.locator('#expenses tr').count() == 2
        page.locator('#filter').select_option('all')
        page.locator('#receipt').fill('75')
        page.locator('#plan').click()
        page.wait_for_function("() => !document.getElementById('run').disabled")
        page.locator('#run').click()
        page.wait_for_function("() => !document.getElementById('accept').hidden && !document.getElementById('accept').disabled", timeout=60000)
        assert page.locator('#test-count').inner_text() == '8/8'
        assert page.locator('#needs-count').inner_text() == '4'
        with page.expect_download() as download:
            page.locator('#export').click()
        exported = json.loads(pathlib.Path(download.value.path()).read_text())
        assert exported['task']['mode'] == 'guided'
        assert len(exported['task']['versions']) == 2
        page.locator('#mode').select_option('recorded')
        assert 'No live run recorded yet' in page.locator('#notice').inner_text()
        assert page.locator('#plan').is_disabled()
        page.locator('#mode').select_option('guided')
        page.locator('#plan').click()
        page.wait_for_function("() => !document.getElementById('run').disabled")
        page.locator('#run').click()
        page.locator('#cancel').click()
        page.wait_for_timeout(700)
        assert page.locator('#artifact-title').inner_text() == 'Cancelled'
        assert page.locator('#accept').is_hidden()
        page.set_viewport_size({'width': 390, 'height': 844})
        page.screenshot(path=str(args.screenshots / 'mobile.png'), full_page=True)
        assert page.evaluate('document.documentElement.scrollWidth <= innerWidth + 1')
        # Restore a legacy Review task whose five passing rows omitted missing receipts.
        # It must not promote the candidate or mutate the accepted baseline.
        if not args.offline_ui:
            page.evaluate('''async () => {
              const core=await import(new URL('core.mjs',location.href));
              const {execute}=await import(new URL('sandbox.mjs',location.href));
              const {save}=await import(new URL('storage.mjs',location.href));
              const rule=core.policy('500');
              const candidate=`function evaluateExpense(expense) {
                if(expense.status==='paid')return {status:'paid',reason:'Already paid',canApprove:false};
                if(expense.amountCents>50000)return {status:'manager_review',reason:'Above threshold',canApprove:false};
                if(expense.receipt===false)return {status:'receipt_required',reason:'Missing receipt',canApprove:false};
                return {status:'eligible',reason:'Eligible',canApprove:true};
              }`;
              const hash=await core.digest(candidate);
              const oldCases=core.requiredCases(rule).filter(test=>test.input.receipt);
              const rows=core.score(oldCases,await execute(candidate,oldCases.map(test=>test.input)),rule);
              if(rows.length!==5||!rows.every(row=>row.pass))throw Error('Legacy fixture changed');
              const task={id:'legacy-test',mode:'live',request:'Approval only',rule,base:core.BASE_SOURCE,
                status:'Review',phase:'Human approval',attempt:1,plan:{summary:'Legacy task',steps:['Implement']},
                candidate,hash,evidence:{hash,rows},versions:[],events:[]};
              await save({schema:1,accepted:core.BASE_SOURCE,task,history:[],layout:false});
            }''')
            page.reload()
            page.wait_for_function("() => document.getElementById('notice').textContent.includes('Candidate violates current policy')")
            assert 'Studio Supply must be eligible' in page.locator('#notice').inner_text()
            assert 'Design Tools must be eligible' in page.locator('#notice').inner_text()
            assert page.locator('#accept').is_hidden()
            # Exercise promotion guard directly as well as the hidden UI control.
            persisted = page.evaluate('''async () => {
              await document.getElementById('accept').onclick();
              const {load}=await import(new URL('storage.mjs',location.href));
              const {BASE_SOURCE}=await import(new URL('core.mjs',location.href));
              const stored=await load();
              return {unchanged:stored.accepted===BASE_SOURCE,status:stored.task.status};
            }''')
            assert persisted == {'unchanged':True,'status':'Blocked'}, persisted
            assert page.locator('#needs-count').inner_text() == '0'
            print('PASS: stale passing Review evidence is blocked by the current policy oracle; acceptance leaves the baseline unchanged.')
        assert not external_requests, external_requests
        assert not errors, errors
        print('PASS: guided boundary repair, receipt rule, human acceptance, persistence, version switch, search/filter, export, honest recording mode, cancellation, both layouts, mobile width.')
        print('Adapter: ' + ('OFFLINE TEST NODE VM — QuickJS and Gemma NOT exercised' if args.offline_ui else 'bundled real browser QuickJS with external requests blocked — Gemma NOT exercised'))
        browser.close()
finally:
    server.shutdown()
