"""Bounded isolated signed upgrade evidence; all subprocesses are task-owned."""
from pathlib import Path
import argparse,hashlib,json,os,re,signal,subprocess,sys,time

def analyze(stdout,stderr,blocks):
 summaries=[json.loads(line[14:]) for line in stderr.splitlines() if line.startswith('DELIVER_STATS ')]
 if len(summaries)!=1:raise ValueError('expected one DELIVER_STATS')
 stats=summaries[0]
 if stats['blocks']!=blocks or stats['requested_blocks']!=blocks or stats['mismatches'] or stats['split']:raise ValueError('incomplete or mismatched run')
 switches=re.findall(r'^UPGRADE h=(\d+): Go switched binaries, app version 9 -> 10$',stdout,re.M)
 if len(switches)!=1:raise ValueError('expected one actual Go v9 to v10 binary switch')
 verified=stats['hl_verified_versions']
 if not stats['hl_preupgrade_required'] or any(verified.get('v9/'+kind,0)<1 for kind in ['collateral','synthetic']):raise ValueError('missing signed pre-upgrade path')
 phases={name:{'blocks':int(b),'delivered':int(n),'go_ok':int(ok),'check_rejected':int(rej)} for name,b,n,ok,rej in re.findall(r'^PHASE (.+): blocks=(\d+) delivered=(\d+) go_ok=(\d+) check_rejected=(\d+)$',stdout,re.M)}
 after=[row for name,row in phases.items() if name.startswith('v10 ')]
 if not after or sum(row['delivered'] for row in after)<1 or sum(row['go_ok'] for row in after)<1:raise ValueError('missing successful finalized v10 phase traffic')
 logs=re.findall(r'(\d+) log-only differences',stderr)
 if len(logs)!=1:raise ValueError('missing log difference summary')
 return {'stats':stats,'switch_height':int(switches[0]),'phases':phases,'log_only_differences':int(logs[0]),'signed_postupgrade_both_paths':all(verified.get('v10/'+kind,0)>0 for kind in ['collateral','synthetic'])}

def main():
 parser=argparse.ArgumentParser(description=__doc__)
 parser.add_argument('--out',type=Path,required=True);parser.add_argument('--binary',type=Path,required=True)
 parser.add_argument('--candidate',required=True);parser.add_argument('--port-offset',required=True,type=int)
 args=parser.parse_args();source=Path.cwd()
 actual=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()
 if actual!=args.candidate:raise ValueError('candidate source differs')
 args.out.mkdir(parents=True,exist_ok=False);records=[];child=None
 def interrupted(signum,_frame):raise KeyboardInterrupt(f'received signal {signum}')
 signal.signal(signal.SIGTERM,interrupted);signal.signal(signal.SIGINT,interrupted)
 summary={'candidate':actual,'status':'running','runs':records,'scope':'50 signed devnet-shaped upgrades; no mainnet-shaped or other qualification claim'}
 def save():(args.out/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
 save()
 try:
  for seed in range(1,51):
   mode='message-id' if seed%2 else 'merkle-root';name=f'upgrade-{seed:02}-{mode}';start=time.monotonic()
   environment={'GO_APPD':'/private/tmp/celestia-rc-46acbcb/go-bin/v9/celestia-appd','GO_APPD_NEXT':'/private/tmp/celestia-rc-46acbcb/go-bin/celestia-appd','FUZZ_DIFF':str(args.binary),'DELIVER_UPGRADE':str(10+seed%30),'DELIVER_IBC':'1','DELIVER_HL':'1','DELIVER_HL_ISM':mode,'DEVNET_PORT_OFFSET':str(args.port_offset),'GOMAXPROCS':'1','GOMEMLIMIT':'512MiB','GO_LOG':str(args.out/(name+'.go.log'))}
   command=[str(source/'crates/fuzz-diff/deliver-diff.sh'),'v9-upgrade','240','6',str(seed)]
   (args.out/(name+'.env.json')).write_text(json.dumps(environment,indent=2)+'\n')
   with (args.out/(name+'.out')).open('wb') as stdout,(args.out/(name+'.err')).open('wb') as stderr:
    child=subprocess.Popen(command,env={**os.environ,**environment},stdin=subprocess.DEVNULL,stdout=stdout,stderr=stderr,start_new_session=True)
    try:code=child.wait(timeout=600)
    except subprocess.TimeoutExpired:
     os.killpg(child.pid,signal.SIGTERM)
     try:child.wait(timeout=15)
     except subprocess.TimeoutExpired:os.killpg(child.pid,signal.SIGKILL);child.wait()
     raise TimeoutError(f'{name} exceeded per-run budget')
   (args.out/(name+'.exit')).write_text(str(code)+'\n')
   if code:raise RuntimeError(f'{name} exit {code}')
   result=analyze((args.out/(name+'.out')).read_text(),(args.out/(name+'.err')).read_text(),240)
   record={'seed':seed,'mode':mode,'seconds':time.monotonic()-start,'exit_code':code,**result,'artifact_sha256':{suffix:hashlib.sha256((args.out/(name+'.'+suffix)).read_bytes()).hexdigest() for suffix in ['out','err','exit','env.json','go.log']}}
   records.append(record);save();print(f'{name}: {result["stats"]["delivered"]} finalized, signed v9 paths checked, one switch, v10 traffic checked',flush=True)
  summary['status']='passed';save()
 except BaseException as error:
  summary.update(status='failed',error=repr(error));save();raise
 finally:
  if child is not None and child.poll() is None:
   os.killpg(child.pid,signal.SIGTERM)
   try:child.wait(timeout=15)
   except subprocess.TimeoutExpired:os.killpg(child.pid,signal.SIGKILL);child.wait()
 return 0

if __name__=='__main__':sys.exit(main())
