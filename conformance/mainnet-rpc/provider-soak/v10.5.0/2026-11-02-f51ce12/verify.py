#!/usr/bin/env python3
"""Check the final candidate's frozen bounded all-API soak, without network I/O."""
from collections import Counter
from datetime import datetime, timedelta
import hashlib
import json
from pathlib import Path
import re

HERE=Path(__file__).resolve().parent
QUERY={'comet-http-status','comet-jsonrpc-history','comet-jsonrpc-proof','comet-ws-subscribe','comet-ws-slow-reconnect','core-grpc-status','core-grpc-history','core-grpc-slow-subscribe','cosmos-grpc-pagination-history','cosmos-grpc-block','rest-pagination-history','rest-node-info'}
BROADCAST={'jsonrpc','uri','websocket','cosmos-grpc','rest','core-grpc'}
def require(ok,message):
    if not ok:raise ValueError(message)
def load(name):return json.loads((HERE/name).read_text())
def digest(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def verify():
    manifest=load('manifest.json');actual={str(p.relative_to(HERE)) for p in HERE.rglob('*') if p.is_file() and p.name!='manifest.json'}
    require(actual==set(manifest['artifacts']),'incomplete soak artifact inventory')
    for name,sha in manifest['artifacts'].items():require(digest(HERE/name)==sha,'changed soak artifact: '+name)
    provenance=load('provider-provenance.json');require(provenance['source_commit']==manifest['candidate_commit'],'wrong candidate source')
    require('commit: '+manifest['candidate_commit'] in provenance['rs_version'],'wrong native build stamp')
    require({k:v['sha256'] for k,v in provenance['binaries'].items()}==manifest['binary_sha256'],'wrong binary identity')
    recipe=provenance['go_build_recipe'];require(recipe['source_commit']=='6013e758d5dceb75827ad191c8d739d7c43cfa04' and recipe['compiler']=='go version go1.26.9 linux/amd64' and recipe['build_tags']==['ledger'],'wrong oracle build')
    for name,sha in provenance['capture_sources_sha256'].items():require(digest(HERE/'capture-sources'/name)==sha,'capture source changed')
    obs=load('mainnet-observation.json');require(obs==provenance['mainnet_observation'],'observation binding differs')
    require(obs['chain_id']=='celestia' and obs['version']=='10.5.0','wrong oracle selection')
    for route in ['status','abci_info']:
        r=obs['responses'][route];require(hashlib.sha256(r['body'].encode()).hexdigest()==r['sha256'],'unbound observation')
    require(0<=provenance['captured_at_unix']-datetime.fromisoformat(obs['observed_at']).timestamp()<=86400,'stale observation')
    report=load('report.json');require(report['final'] and report['elapsed_s']>=300 and report['heights']>=80,'insufficient bounded duration/heights')
    require(not report['failures'] and not report['stalls'] and not report['log_hits'],'soak failed health checks')
    nodes=report['nodes'];require([n['kind'] for n in nodes]==['go','go','rs','rs','rs'] and [n['validator'] for n in nodes]==[True,True,True,True,False],'wrong topology')
    rows=[json.loads(line) for line in (HERE/'hashes.jsonl').read_text().splitlines()]
    require([r['h'] for r in rows]==list(range(1,len(rows)+1)) and len(rows)>=report['heights'],'hash observations have gaps')
    require(report['blocks']['count']==len(rows) and report['blocks']['txs']==sum(r['ntx'] for r in rows) and report['blocks']['max_bytes']==max(r['size'] for r in rows),'block summary differs')
    require([n['checked_upto'] for n in nodes]==manifest['checked_upto'] and min(n['checked_upto'] for n in nodes)>=max(80,len(rows)-3),'insufficient or changed consistency coverage')
    provider=report['metrics']['provider'];require(provider['enabled'] and provider['disabled_scenarios']==provenance['coverage_limitations'],'limitations disappeared')
    ops=provider['operations'];expected={*(f'n{n}:{q}' for n in range(5) for q in QUERY),*(f'n{n}:broadcast:{t}' for n in range(5) for t in BROADCAST)}
    require(set(ops)==expected,'missing node/transport operation')
    for name,result in ops.items():
        require(result['requested']==result['completed']>0 and not result['errors'] and not result['admission_unknown'],'unfinished/failed provider operation')
        if ':broadcast:' in name:require(result['accepted']==result['completed'],'broadcast was not accepted')
    events=[json.loads(line) for line in (HERE/'events.log').read_text().splitlines()];queries=[e for e in events if e['ev']=='provider-result'];broadcasts=[e for e in events if e['ev']=='provider-broadcast']
    require(all(q['ok'] and not q.get('error') for q in queries),'provider query failed')
    require(all(b['code']==0 and (b['transport']!='core-grpc' or b.get('tx_result_code')==0) for b in broadcasts),'broadcast admission/execution failed')
    require(Counter((q['node'],q['operation']) for q in queries)==Counter({(n,q):ops[f'n{n}:{q}']['completed'] for n in range(5) for q in QUERY}),'query event matrix differs')
    require(Counter((b['node'],b['transport']) for b in broadcasts)==Counter({(n,t):ops[f'n{n}:broadcast:{t}']['completed'] for n in range(5) for t in BROADCAST}),'broadcast event matrix differs')
    for name in ['bank','pfb','staking']:
        row=report['metrics'][name];require(not row['lost'] and not row['failed_exec'],'lost or failed indexed workload transaction')
    require(sum(r['completed'] for r in ops.values())==manifest['provider_operations']==960,'provider count changed')
    require(report['elapsed_s']==manifest['elapsed_s']==312 and report['heights']==manifest['height_advances']==112,'frozen duration/height count changed')
    for name in ['rss.csv','disk.csv','mem.jsonl']:require((HERE/name).stat().st_size>0,'missing resource samples')
    print('Verified final-candidate all-API soak: 312s, 112 height advances, 960 provider operations, zero client/consistency failures; declared bounded exclusions apply')
if __name__=='__main__':verify()
