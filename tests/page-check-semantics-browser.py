#!/usr/bin/env python3
"""Real DOM regression for any/all/first node assertions; no model or remote calls."""
import argparse
import functools
import http.server
import json
import pathlib
import shutil
import threading
from playwright.sync_api import sync_playwright
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
try:
    with sync_playwright() as p:
        browser = p.chromium.launch(executable_path=args.browser, headless=True)
        page = browser.new_page()
        page.route('https://**/*', lambda route: route.abort())
        page.goto(f'http://127.0.0.1:{server.server_port}/demo/')
        result = page.evaluate('''async()=>{
            const {runPageChecks}=await import('./page-runtime.mjs');
            const assertion={selector:'thead th',property:'text',operator:'equals',expected:'Date'};
            const checks=[
                {name:'Any node is the default',assertions:[assertion]},
                {name:'Every node must match with all',assertions:[{...assertion,all:true}]},
                {name:'Only the first node with first',assertions:[{...assertion,first:true}]},
                {name:'Count still counts every match',assertions:[{selector:'thead th',property:'count',operator:'equals',expected:2}]},
                {name:'No matched node cannot pass',assertions:[{...assertion,selector:'.absent'}]},
                {name:'Actions still target first match',steps:[{action:'click',selector:'button'}],assertions:[{selector:'#clicked',property:'text',operator:'equals',expected:'first'}]}
            ];
            return await runPageChecks({html:'<table><thead><tr><th>Merchant</th><th>Date</th></tr></thead></table><button id="first">First</button><button id="second">Second</button><output id="clicked"></output>',css:'',js:`for(const button of document.querySelectorAll('button'))button.onclick=()=>document.querySelector('#clicked').textContent=button.id;`},checks);
        }''')
        assert [row['pass'] for row in result['rows']] == [True, False, False, True, False, True, True], result
        assert result['rows'][0]['actual'][0]['actual'] == ['Merchant','Date'], result
        assert result['rows'][2]['actual'][0]['actual'] == ['Merchant'], result
        assert result['errors'] == [], result
        disabled = page.evaluate('''async()=>{
            const {runPageChecks}=await import('./page-runtime.mjs');
            const bundle={html:'<button class="disabled" disabled>Disabled</button><fieldset disabled><legend><input id="legend" class="enabled"></legend><input class="disabled"><legend><input class="disabled"></legend><button class="disabled">Inherited</button></fieldset><select disabled class="disabled"><option>Choice</option></select><select><optgroup disabled class="disabled"><option class="disabled">Inherited option</option></optgroup><option class="enabled">Enabled</option></select><textarea class="disabled" disabled></textarea><button id="toggle">Disable me</button><button class="enabled" aria-disabled="true">ARIA only</button><div id="fake" disabled aria-disabled="true">Not a control</div>',css:'',js:`const button=document.querySelector('#toggle');button.onclick=()=>button.setAttribute('disabled','');Object.defineProperty(button,'disabled',{get:()=>true});Element.prototype.matches=()=>true;`};
            const assertion=(selector,expected)=>({selector,property:'disabled',operator:'equals',expected});
            return await runPageChecks(bundle,[
                {name:'Native and inherited disabled controls',assertions:[{...assertion('.disabled',true),all:true}]},
                {name:'Legend exception, enabled option and ARIA-only button',assertions:[{...assertion('.enabled',false),all:true},assertion('#toggle',false)]},
                {name:'Non-control cannot pass true',assertions:[assertion('#fake',true)]},
                {name:'Non-control cannot pass false',assertions:[assertion('#fake',false)]},
                {name:'Missing control fails',assertions:[assertion('#absent',false)]},
                {name:'Real action disables the control',steps:[{action:'click',selector:'#toggle'}],assertions:[assertion('#toggle',true)]}
            ]);
        }''')
        assert [row['pass'] for row in disabled['rows']] == [True,True,False,False,False,True,True], disabled
        assert disabled['rows'][1]['actual'][-1]['actual']==[False], disabled
        assert 'native form control' in disabled['rows'][2]['actual'][0]['actual'][0]['error'], disabled
        assert disabled['errors']==[], disabled
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps({'node_semantics':result,'native_disabled':disabled}, indent=2)+'\n')
        print('PASS: real DOM default-any, all, explicit-first, count, missing-node, native/inherited disabled, action and prototype-hardening semantics')
        browser.close()
finally:
    server.shutdown()
