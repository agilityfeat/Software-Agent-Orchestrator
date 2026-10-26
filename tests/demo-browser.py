#!/usr/bin/env python3
"""Compatibility entry point for the current full-page browser smoke.

Runs the independent workspace suite against the production isolated DOM runtime.
Its synthetic model adapter is NON_ACCEPTANCE evidence for actual Gemma inference.
Use --screenshots DIR to retain its screenshots and report.
"""
import pathlib
import runpy
import sys

if '--offline-ui' in sys.argv:
    raise SystemExit('The full-page demo uses a real browser DOM runtime. The former QuickJS --offline-ui adapter is no longer applicable; omit that flag.')
target=pathlib.Path(__file__).with_name('page-workspace-browser.py')
sys.argv[0]=str(target)
runpy.run_path(str(target),run_name='__main__')
