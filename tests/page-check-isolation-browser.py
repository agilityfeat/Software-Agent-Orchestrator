#!/usr/bin/env python3
"""Real DOM regression: top-level cases start fresh; actions within a case share state."""
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
            const bundle={html:'<button id="toggle">Show details</button><p id="detail" hidden>Details</p><output id="count">0</output>',css:'',js:`
                document.querySelector('#count').textContent=localStorage.getItem('clicks')||'0';
                document.querySelector('#toggle').onclick=()=>{const detail=document.querySelector('#detail');detail.hidden=!detail.hidden;const clicks=Number(localStorage.getItem('clicks')||0)+1;localStorage.setItem('clicks',clicks);document.querySelector('#count').textContent=clicks;};
            `};
            const hidden={selector:'#detail',property:'visible',operator:'equals',expected:false};
            const click={action:'click',selector:'#toggle'};
            const checks=[
                {name:'Starts hidden',assertions:[hidden]},
                {name:'One click shows it',steps:[click],assertions:[{...hidden,expected:true},{selector:'#count',property:'text',operator:'equals',expected:'1'}]},
                {name:'Later case starts hidden and empty again',assertions:[hidden,{selector:'#count',property:'text',operator:'equals',expected:'0'}]},
                {name:'Two clicks in one case toggle back',steps:[click,click],assertions:[hidden,{selector:'#count',property:'text',operator:'equals',expected:'2'}]},
                {name:'A missing selector still fails',assertions:[{selector:'.missing',property:'count',operator:'equals',expected:1}]}
            ];
            const isolated=await runPageChecks(bundle,checks);
            const broken=await runPageChecks({...bundle,js:bundle.js+';throw new Error("Deliberate source failure")'},[{name:'First broken frame',assertions:[hidden]},{name:'Second broken frame',assertions:[hidden]}]);
            const abort=new AbortController();let abortFailure;setTimeout(()=>abort.abort(),250);
            try{await runPageChecks(bundle,[{name:'Abort after setup',steps:[{action:'wait',ms:500}],assertions:[hidden]},{name:'Must not run',assertions:[hidden]}],{signal:abort.signal,onStorage:()=>{}});}catch(error){abortFailure=error.message;}
            return {isolated,broken,abortFailure,frames:document.querySelectorAll('iframe[title="Application preview"]').length};
        }''')
        assert [row['pass'] for row in result['isolated']['rows']] == [True,True,True,True,False,True], result
        assert result['isolated']['observations']['storage'] == {}, result
        assert len(result['broken']['rows']) == 3 and result['broken']['rows'][-1]['pass'] is False, result
        assert len(result['broken']['errors']) == 2, result
        assert result.get('abortFailure'), result
        assert result['frames'] <= 1, result  # The main visible app preview may exist.
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(result, indent=2)+'\n')
        print('PASS: fresh DOM/storage per case, sequential actions within case, ordered rows, count failures and aggregated source errors')
        browser.close()
finally:
    server.shutdown()
