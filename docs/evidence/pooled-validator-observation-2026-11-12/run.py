from pathlib import Path
from datetime import datetime,timezone
import hashlib,json,select,signal,subprocess,sys,time,urllib.parse
root=Path(__file__).resolve().parent
network=sys.argv[1];assert network in ('mocha','mainnet')
output=root/network;output.mkdir()
source=json.loads((root/'source.json').read_text())
for name,expected in source['tool_sha256'].items():assert hashlib.sha256((root/'tools'/name).read_bytes()).hexdigest()==expected
relay=root/'rpc-relay';assert hashlib.sha256(relay.read_bytes()).hexdigest()==source['relay_binary_sha256']
config=json.loads((root/f'{network}.json').read_text())
metadata={**source,'network':network,'started_at_utc':datetime.now(timezone.utc).isoformat(),'upstream':config['reference_rpc'],'status':'starting'}
def save():(output/'run.json').write_text(json.dumps(metadata,indent=2)+'\n')
inputs={str(path):hashlib.sha256(path.read_bytes()).hexdigest() for path in [Path(__file__),relay,*(root/'tools'/name for name in source['tool_sha256'])]}
metadata['input_sha256']=inputs
save();child=None;proxy=None;code=1
def interrupted(signum,_frame):raise KeyboardInterrupt(f'received signal {signum}')
signal.signal(signal.SIGTERM,interrupted)
signal.signal(signal.SIGINT,interrupted)
try:
 with (output/'relay.stderr.log').open('wb') as err:
  proxy=subprocess.Popen([str(relay),'--upstream',config['reference_rpc'],'--listen','127.0.0.1:0','--rps','4','--connections','4','--duration','73h'],stdout=subprocess.PIPE,stderr=err,text=True,stdin=subprocess.DEVNULL)
  assert select.select([proxy.stdout],[],[],10)[0],'relay ready timeout'
  ready=json.loads(proxy.stdout.readline());url=urllib.parse.urlsplit(ready['listen'])
  assert ready['pid']==proxy.pid and url.hostname=='127.0.0.1' and url.scheme=='http' and url.port>0
  metadata['relay']=ready;config['reference_rpc']=ready['listen']
  (output/'config.json').write_text(json.dumps(config,indent=2)+'\n')
  command=['python3',str(root/'tools/observe_validator.py'),'--config',str(output/'config.json'),'--output',str(output/'receipts'),'--seconds','259500','--interval','5']
  with (output/'observer.log').open('wb') as log:
   child=subprocess.Popen(command,stdin=subprocess.DEVNULL,stdout=log,stderr=subprocess.STDOUT)
   metadata.update({'command':command,'child_pid':child.pid,'status':'running'});save()
   deadline=time.monotonic()+259680
   while child.poll() is None:
    if proxy.poll() is not None:raise RuntimeError('reference relay exited before collector')
    if time.monotonic()>deadline:raise TimeoutError('collector exceeded bounded lifetime')
    time.sleep(1)
   code=child.returncode
except BaseException as error:
 metadata['error']=repr(error)
 if isinstance(error,KeyboardInterrupt):code=130
finally:
 for process in [child,proxy]:
  if process is not None and process.poll() is None:
   process.send_signal(signal.SIGTERM)
   try:process.wait(timeout=15)
   except subprocess.TimeoutExpired:process.kill();process.wait()
 unchanged=all(Path(path).is_file() and hashlib.sha256(Path(path).read_bytes()).hexdigest()==expected for path,expected in inputs.items())
 if not unchanged:code=1
 metadata['inputs_unchanged']=unchanged
 metadata.update({'status':'finished' if code==0 else 'failed','child_exit_code':code,'finished_at_utc':datetime.now(timezone.utc).isoformat()});save()
sys.exit(code)
