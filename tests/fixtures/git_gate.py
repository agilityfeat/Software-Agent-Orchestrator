#!/usr/bin/python3
"""Synthetic preflight barrier; forwards only to local Git. No provider/network."""
import os,pathlib,sys,time
root=pathlib.Path(__file__).parent
if 'merge-base' in sys.argv[1:]:
 counter=root/'git-preflight-count'
 count=int(counter.read_text())+1 if counter.exists() else 1
 counter.write_text(str(count))
 # First ancestry check verifies prepare; second ends Coder preflight.
 if count==2:
  (root/'preflight-waiting').write_text('synthetic barrier reached\n')
  deadline=time.monotonic()+8
  while (root/'hold-preflight').exists() and time.monotonic()<deadline:time.sleep(.005)
os.execv('/usr/bin/git',['/usr/bin/git',*sys.argv[1:]])
