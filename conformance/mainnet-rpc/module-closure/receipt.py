#!/usr/bin/env python3
"""Bind fresh inputs, exact source files and binaries without changing Go answers."""
import datetime
import hashlib
import json
import os
import pathlib
import subprocess
import sys

root,out=map(pathlib.Path,sys.argv[1:3]);workload=sys.argv[3]
def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
paths=subprocess.check_output(['git','ls-files','--cached','--others','--exclude-standard'],cwd=root,text=True).splitlines()
sources={p:sha(root/p) for p in sorted(set(paths)) if (root/p).is_file() and ((p.startswith('crates/') and '/tests/' not in p) or p.startswith('conformance/devnet/') or p.startswith('conformance/rpccorpus/recorder/') or (p.startswith('conformance/mainnet-rpc/module-closure/proofcheck/') or p in ['conformance/mainnet-rpc/module-closure/capture.sh','conformance/mainnet-rpc/module-closure/record.sh','conformance/mainnet-rpc/module-closure/receipt.py']) or p in ['Cargo.toml','Cargo.lock','rust-toolchain.toml'])}
cap=out/'capture'
tools={}
for name in ['celestia-appd','cometbft','abci-recorder','devnet-oracle','rpc-recorder','local-signer','populated-matrix','proofcheck']+(['rly'] if workload=='ibc' else []):
 p=pathlib.Path(os.environ['DEVNET_BIN'])/name
 tools[name]={'sha256':sha(p),'build_info':subprocess.check_output(['go','version','-m',str(p)],text=True)}
for name,key in [('populated_requests','POPULATED_REQUESTS_BIN')]+([('fuzz-diff','FUZZ_DIFF')] if workload=='utf8' else []):
 tools[name]={'sha256':sha(pathlib.Path(os.environ[key]))}
files={str(p.relative_to(out)):sha(p) for p in sorted(out.rglob('*')) if p.is_file() and p.name!='receipt.json'}
receipt={'schema_version':1,'captured_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'scenario':out.name,'workload':workload,'source_commit':subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip(),'source_files':sources,'tools':tools,'rustc':subprocess.check_output(['rustc','--version','--verbose'],text=True),'files':files,'limits':['live local-devnet timestamps and generated workload keys are retained as signed public inputs; replay is deterministic','Go and Rust diagnostic source frames and explicitly inventoried Go map-order lists are normalized','proofs rejected by the official Go verifier remain explicit negative results']}
(cap/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n')
