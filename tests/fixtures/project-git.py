#!/usr/bin/python3
import sys,subprocess,pathlib
args=sys.argv[1:]
if 'clone' in args:
 if (pathlib.Path(__file__).resolve().parent/'slow-clone').exists():
  import time,os
  (pathlib.Path(__file__).resolve().parent/'clone-started').write_text(str(os.getpid()))
  time.sleep(60)
 args=[a.replace('protocol.file.allow=never','protocol.file.allow=always') for a in args]
 i=args.index('--')+1; origin=args[i]; name=origin.rstrip('/').split('/')[-1].removesuffix('.git');source=pathlib.Path(__file__).resolve().parent/name
 result=subprocess.run(['/usr/bin/git',*args[:i],str(source),*args[i+1:]])
 if result.returncode==0: subprocess.check_call(['/usr/bin/git','-C',args[-1],'remote','set-url','origin',origin])
 sys.exit(result.returncode)
sys.exit(subprocess.run(['/usr/bin/git',*args]).returncode)
