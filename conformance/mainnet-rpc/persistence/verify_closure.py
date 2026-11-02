#!/usr/bin/env python3
"""Validate the bounded persistent API, reconnect, upgrade and snapshot evidence."""
import base64
from collections import Counter
from datetime import datetime, timedelta
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import tomllib

HERE=Path(__file__).resolve().parent
ROOT=HERE.parents[2]
DATA=HERE/'closure-evidence'
PROFILES=('retain-kv','prune-kv','retain-null','prune-null')
PHASES=('before-restart','after-restart','after-pruning-boundary')
STABLE_HEADERS={'content-type','x-cosmos-block-height','grpc-metadata-x-cosmos-block-height','grpc-metadata-content-type'}
FRAMES=re.compile(r' \[[^\]]*\.go:[0-9]+\]')
def require(ok,message):
    if not ok:raise ValueError(message)
def load(path):return json.loads(path.read_text())
def digest(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def module(name,path):
    spec=importlib.util.spec_from_file_location(name,path);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m
def canonical(answer):
    md=lambda key:{k.lower():v for k,v in (answer.get(key) or {}).items() if k.lower() in STABLE_HEADERS}
    body=answer.get('body')
    if isinstance(body,dict) and isinstance(body.get('message'),str):body={**body,'message':FRAMES.sub('',body['message'])}
    return {'code':answer['code'],'message':FRAMES.sub('',answer.get('message','')),'value_hex':answer.get('value_hex',''),'headers':md('headers'),'trailers':md('trailers'),'body':body}
def archive(parser,path):
    frames=parser.archive_frames(path);require(len(frames)%2==0,'truncated archive');headers={}
    for index in range(0,len(frames),2):
        h=parser.block_header(frames[index]);height=index//2+1
        require(h['height']==parser.commit_height(frames[index+1])==height,'archive height gap');headers[height]=h
    return headers
def comet(capture,result,headers):
    require(not result['errors'] and len(result['cases'])==40,'incomplete Comet capture')
    require(result['comparison']=='decoded-json-exact','Comet comparison changed')
    require(headers[result['height']+1]['app_hash'].upper()==result['matched_app_hash'].upper(),'wrong matched query root')
    for row in result['cases']:
        require(row['outcome']=='match' and capture.comparable(row['go'])==capture.comparable(row['rs']),'Comet mismatch')
        for role in ('go','rs'):require(hashlib.sha256(row[role]['body'].encode()).hexdigest()==row[role]['body_sha256'],'response hash mismatch')
    statuses=[json.loads(result[role+'_status']['body'])['result'] for role in ('go','rs')]
    tips=[int(s['sync_info']['latest_block_height']) for s in statuses]
    require(tips[0]==tips[1] and all(s['node_info']['network']==result['chain_id'] for s in statuses),'provider tips/chains differ')
    return tips[0]
def api(result,tip,height):
    require(result['tip_height']==tip and result['historical_height']==height,'API height binding differs')
    pairs=result['pairs'];require(len(pairs)==84,'incomplete API matrix')
    seen=Counter()
    for pair in pairs:
        i=pair['input'];key=(i['Name'],i['Height'],pair['transport']);seen[key]+=1
        require(pair['match'] and canonical(pair['Go'])==canonical(pair['RS']),'API response/metadata mismatch')
        if pair['transport']=='grpc':require(pair['Go']['code'] not in (4,14),'transport unavailable was counted as a match')
    expected=Counter((name,h,t) for name in ['balance','invalid-address','account','offset','reverse','invalid-cursor','empty-page'] for h in ['0','1',str(height),str(tip+100),'-1','garbage'] for t in ['grpc','rest'])
    require(seen==expected,'missing height/error branch')
def verify():
    manifest=load(DATA/'manifest.json');require(manifest['scope']=='bounded-ordered-persistent-provider-closure','scope changed')
    actual={str(p.relative_to(DATA)) for p in DATA.rglob('*') if p.is_file() and p.name!='manifest.json'}
    require(actual==set(manifest['artifacts']),'incomplete public artifact inventory')
    for name,sha in manifest['artifacts'].items():require(digest(DATA/name)==sha,'changed persistence artifact: '+name)
    for name,sha in manifest['sources'].items():require(digest(ROOT/name)==sha,'changed verifier/source: '+name)
    parser=module('closure_blocks',ROOT/'conformance/mainnet-rpc/populated-governance/verify.py')
    capture=module('closure_comet',HERE/'capture.py')
    counts={'comet':0,'api':0,'reconnect_events':0}
    for profile in PROFILES:
        batch=DATA/manifest['profiles'][profile];directory=batch/profile;receipts=load(batch/'build-receipts.json')
        for name,sha in receipts['sources'].items():require(digest(batch/'sources'/name)==sha,'unbound archived capture source')
        require((batch/'sources/conformance/mainnet-rpc/persistence/capture.py').read_bytes()==(HERE/'capture.py').read_bytes(),'review changed Comet comparator')
        require('go1.26.9' in receipts['tools']['persistent-api']['build_info'],'wrong client compiler')
        for name,commit in [('celestia-appd','6013e758d5dceb75827ad191c8d739d7c43cfa04'),('cometbft','a5537d84a008d566abc503d2ea41e30f2301403d')]:
            receipt=load(batch/f'{name}.build.json');require(receipt['source_commit']==commit and receipt['compiler']=='go version go1.26.9 linux/amd64','wrong source-built oracle')
        obs=load(batch/'mainnet-observation.json');observed=datetime.fromisoformat(obs['observed_at'])
        for route in ['status','abci_info']:
            r=obs['responses'][route];require(hashlib.sha256(r['body'].encode()).hexdigest()==r['sha256'],'unbound mainnet observation')
        require(obs['version']=='10.5.0' and obs['chain_id']=='celestia','wrong selected release')
        for node in range(3):
            config=tomllib.loads((directory/f'config-{node}.toml').read_text());app=tomllib.loads((directory/f'app-{node}.toml').read_text())
            require(config['tx_index']['indexer']==('null' if profile.endswith('null') else 'kv'),'wrong index profile')
            require(not config['storage']['discard_abci_responses'] and app['min-retain-blocks']==0,'wrong retention profile')
            require(app['api']['enable'] and app['grpc']['enable'],'API listeners were not enabled')
            require(app['pruning']==('custom' if profile.startswith('prune') else 'nothing'),'wrong app pruning')
            if profile.startswith('prune'):require(int(app['pruning-keep-recent'])==int(app['pruning-interval'])==10,'wrong pruning boundary')
        headers=archive(parser,directory/'blocks.bin.gz');previous=0
        for phase in PHASES:
            result=load(directory/f'{phase}.json');tip=comet(capture,result,headers)
            require(tip>previous,'providers did not advance after restart');previous=tip
            require(timedelta(0)<=datetime.fromisoformat(result['captured_at'])-observed<=timedelta(hours=24),'stale selection')
            api(load(directory/f'{phase}-api.json'),tip,result['height']);counts['comet']+=40;counts['api']+=84
            if phase=='after-pruning-boundary':
                row=next(r for r in result['cases'] if r['method']=='abci_query' and r['params']['height']==result['height'])
                response=json.loads(row['go']['body'])['result']['response'];require((response['code']!=0)==profile.startswith('prune'),'pruning boundary not crossed')
        sockets=load(directory/'reconnect.json');require(len(sockets)==2,'missing reconnect provider')
        events=[]
        for socket in sockets:
            require(not socket.get('error') and socket['disconnect'] and socket['initial_ack']['result']==socket['reconnect_ack']['result']=={},'reconnect failed')
            require(len(socket['events'])==len(socket['heights'])==3,'incomplete reconnect delivery')
            require(list(map(int,socket['heights']))==sorted(set(map(int,socket['heights']))),'event order changed')
            byheight={}
            for event in socket['events']:
                value=event['result']['data']['value'];header=value['block']['header'];height=int(header['height'])
                require(header['chain_id']==headers[height]['chain_id'] and header['app_hash'].lower()==headers[height]['app_hash'],'wrong delivered block root')
                # Go's live SDK result allocates an empty list; Rust retains nil.
                if value['result_finalize_block']['validator_updates'] is None:value['result_finalize_block']['validator_updates']=[]
                byheight[height]=(value,event['result']['events']);counts['reconnect_events']+=1
            events.append(byheight)
        require(events[0].keys()==events[1].keys() and events[0]==events[1],'reconnected payloads differ beyond declared empty-list allocation')
    require(counts=={'comet':480,'api':1008,'reconnect_events':24},'incomplete persistence coverage')
    historical=DATA/'historical';selection=load(historical/'selection.json');release=load(historical/'release.json')
    require(release['archive_sha256']+'  '+release['asset'] in (historical/'release-checksums.txt').read_text(),'release checksum not published')
    require((historical/'multiplexer-build-info.txt').read_text().splitlines()[0].endswith('go1.26.6'),'wrong official compiler')
    require(selection['info']['version']=='10.5.0' and selection['info']['app_version']=='10','historical oracle is not current release')
    versions=selection['versions'];require(versions[0][1]==9 and versions[-1][1]==10,'no v9-to-v10 upgrade')
    upgrade=selection['upgrade_height'];require(selection['heights']==sorted({1,upgrade-1,upgrade,upgrade+1,selection['tip']-1}),'missing upgrade boundary')
    headers=archive(parser,historical/'blocks.bin.gz')
    for height in selection['heights']:
        result=load(historical/f'height-{height}-comet.json');tip=comet(capture,result,headers);api(load(historical/f'height-{height}-api.json'),tip,height)
    sync=DATA/'state-sync';summary=load(sync/'rust-summary.json');trusted=load(sync/'trusted-block.json')['result']['block']['header']
    require(summary['ok'] and not summary['failures'] and summary['chunks_served']>0 and summary['snapshot_list_requests']>0,'state sync did not pass both directions')
    require(summary['restored_height']==summary['snapshot_height']==int(trusted['height'])-1,'snapshot height not bound')
    require(summary['restored_app_hash']==summary['trusted_app_hash']==trusted['app_hash'],'snapshot root differs')
    go=load(sync/'go-info.json')['result']['response'];require(int(go['last_block_height'])==summary['snapshot_height'] and base64.b64decode(go['last_block_app_hash']).hex().upper()==summary['restored_app_hash'],'Go did not restore the Rust-served root')
    chunk=sync/'rust-snapshots'/str(summary['snapshot_height'])/'3/0';require(digest(chunk)==summary['served_snapshot'][2],'snapshot chunk differs')
    print('Verified 480 persistent Comet + 1008 gRPC/REST pairs, 24 reconnect events, 620 historical-multiplexer pairs and bidirectional state sync; ordered startup and declared wire/retention limits apply')
if __name__=='__main__':verify()
