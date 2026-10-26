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
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(result, indent=2)+'\n')
        print('PASS: real DOM default-any, all, explicit-first, count, missing-node and action-first semantics')
        browser.close()
finally:
    server.shutdown()
