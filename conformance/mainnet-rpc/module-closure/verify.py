#!/usr/bin/env python3
"""Verify the frozen module campaign, public inputs, roots and paired artifacts."""
import base64
from collections import Counter
from datetime import datetime, timedelta
import gzip
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import tarfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
WORKLOADS = {'blob','staking-lifecycle','gov','ibc','authz-feegrant','vesting','hyperlane','fibre','zkism-forwarding','utf8'}
TRANSPORTS = {'abci','comet_rpc','cosmos_grpc','rest'}
SORTED = {
 '/cosmos.distribution.v1beta1.Query/DelegationTotalRewards',
 '/cosmos.params.v1beta1.Query/Subspaces',
 '/cosmos.base.reflection.v1beta1.ReflectionService/ListAllInterfaces',
 '/cosmos.base.reflection.v1beta1.ReflectionService/ListImplementations',
 *('/cosmos.base.reflection.v2alpha1.ReflectionService/'+method for method in ['GetAuthnDescriptor','GetCodecDescriptor','GetQueryServicesDescriptor','GetTxDescriptor']),
}
def require(condition, message):
    if not condition: raise ValueError(message)
def load(path): return json.loads(path.read_text())
def digest(path): return hashlib.sha256(path.read_bytes()).hexdigest()
def module(path):
    spec=importlib.util.spec_from_file_location('closure_archive',path)
    result=importlib.util.module_from_spec(spec);spec.loader.exec_module(result);return result
def proto_fields(data):
    def varint(at):
        value=0
        for shift in range(0,70,7):
            require(at<len(data),'truncated protobuf');byte=data[at];at+=1;value|=(byte&127)<<shift
            if byte<128:return value,at
        raise ValueError('overlong protobuf')
    out={};at=0
    while at<len(data):
        tag,at=varint(at);wire=tag&7
        if wire==0:value,at=varint(at)
        elif wire==2:
            size,at=varint(at);value=data[at:at+size];require(len(value)==size,'truncated bytes');at+=size
        elif wire in (1,5):
            size=8 if wire==1 else 4;value=data[at:at+size];require(len(value)==size,'truncated fixed');at+=size
        else:raise ValueError('invalid wire type')
        out.setdefault(tag>>3,[]).append(value)
    return out
def verify():
    manifest=load(HERE/'evidence/manifest.json')
    require(manifest['scope']=='frozen-populated-four-transport-comparison','scope changed')
    for group in ['artifacts','sources']:
        for name,sha in manifest[group].items():
            path=(ROOT/name).resolve();require(path.is_relative_to(ROOT) and path.is_file(),'invalid artifact path')
            require(digest(path)==sha,f'changed {group}: {name}')
    actual={str(p.relative_to(ROOT)) for scenario in WORKLOADS for p in (ROOT/'conformance/fixtures/devnet'/('v105-closure-'+scenario)).rglob('*') if p.is_file()}
    actual|={str(p.relative_to(ROOT)) for p in (HERE/'evidence').rglob('*') if p.is_file() and p.name!='manifest.json'}
    require(actual==set(manifest['artifacts']),'incomplete artifact inventory')
    require(set(load(HERE/'campaigns.json'))=={'v105-closure-'+x for x in WORKLOADS},'missing module campaign')
    parser=module(ROOT/'conformance/mainnet-rpc/populated-governance/verify.py')
    with tarfile.open(HERE/'evidence/collector-source.tar.gz') as archive:
        captured_sources={item.name:hashlib.sha256(archive.extractfile(item).read()).hexdigest() for item in archive if item.isfile()}
    critical=['crates/devnet-conformance/examples/populated_requests.rs','crates/devnet-conformance/src/querycorpus.rs','conformance/rpccorpus/recorder/cmd/populated-matrix/main.go','conformance/rpccorpus/recorder/cmd/local-signer/main.go','conformance/mainnet-rpc/module-closure/proofcheck/main.go','conformance/mainnet-rpc/module-closure/capture.sh']
    with tarfile.open(HERE/'evidence/candidate-source.tar.gz') as archive:
        name='crates/devnet-conformance/tests/populated_module_matrix.rs'
        compiled=archive.extractfile(name).read().decode()
        current=(ROOT/name).read_text()
        identity_adapter='''    let expected = expected_bytes(case, transport);
    // The populated capture is retained v10.5 evidence. Accept only the
    // declared compatibility identity change; transport envelopes are
    // compared field by field by their callers.
    if path == "/app/version"
        && expected == b"10.5.0"
        && bytes == app::COMPATIBILITY_VERSION.as_bytes()
    {
        return;
    }
'''
        require(current.count(identity_adapter)==1,'native version adapter changed')
        current=current.replace(identity_adapter,'')
        adapted_value='canonical(bytes) == canonical(&expected)'
        require(current.count(adapted_value)==1,'native version comparison changed')
        current=current.replace(adapted_value,
                                'canonical(bytes) == canonical(&expected_bytes(case, transport))')
        current='\n'.join(line for line in current.split('\n') if not line.startswith('#[allow(clippy::collapsible_if)]') and not line.startswith('#[allow(clippy::too_many_lines, clippy::result_large_err)]'))
        require(current==compiled,'native test changed beyond declared lint annotations')
    totals=Counter();proofs=Counter();heights=0;methods=set();differences=[]
    for workload in sorted(WORKLOADS):
        scenario='v105-closure-'+workload;directory=ROOT/'conformance/fixtures/devnet'/scenario;cap=directory/'capture'
        receipt=load(cap/'receipt.json');requests=load(cap/'requests.json');freeze=load(cap/'freeze.json')
        go=json.loads(gzip.decompress((cap/'transports.json.gz').read_bytes()))
        native=json.loads(gzip.decompress((HERE/'evidence'/f'{scenario}-rust.json.gz').read_bytes()))
        require(receipt['workload']==workload and receipt['scenario']==scenario,'wrong receipt')
        for source in critical:require(receipt['source_files'][source]==captured_sources[source],'collector source changed')
        for name,sha in receipt['files'].items():require(digest(directory/name)==sha,'changed Go input/response')
        require(go['tip_height']==requests['tip_height']==freeze['tip_height']==native['tip_height'],'tip mismatch')
        require(freeze['stable_samples']>=3 and go['query_height']==requests['primary_height']==go['tip_height']-1,'capture was not frozen')
        tip=go['tip_height'];heights+=tip
        require(native['matched_heights']==list(range(1,tip+1)),'incomplete matched-state replay')
        genesis=load(directory/'genesis.json');require(genesis['chain_id']==go['chain_id']==native['chain_id'],'chain mismatch')
        require(native['app_hash']==go['app_hash'] and native['query_height']==go['query_height'],'root/height binding differs')
        for name,commit in [('celestia-appd','6013e758d5dceb75827ad191c8d739d7c43cfa04'),('cometbft','a5537d84a008d566abc503d2ea41e30f2301403d')]:
            build=load(cap/f'{name}-build.json')
            require(build['source_commit']==commit and build['compiler']=='go version go1.26.9 linux/amd64','wrong oracle')
            require(build['binary_sha256']==receipt['tools'][name]['sha256'],'oracle binary differs')
            if name=='celestia-appd':require(build['build_tags']==['ledger'],'wrong app build')
        obs=load(cap/'mainnet-observation.json');observed=datetime.fromisoformat(obs['observed_at']);captured=datetime.fromisoformat(receipt['captured_at'])
        require(timedelta(0)<=captured-observed<=timedelta(hours=24),'stale observation')
        for route in ['status','abci_info']:
            row=obs['responses'][route];require(hashlib.sha256(row['body'].encode()).hexdigest()==row['sha256'],'unbound observation')
        require(obs['chain_id']=='celestia' and obs['version']=='10.5.0','wrong selected release')
        frames=parser.archive_frames(directory/'blocks.bin.gz');require(len(frames)==2*tip,'incomplete block archive')
        roots={}
        for index in range(0,len(frames),2):
            header=parser.block_header(frames[index]);height=index//2+1
            require(header['height']==parser.commit_height(frames[index+1])==height and header['chain_id']==go['chain_id'],'block identity mismatch')
            roots[height-1]=header['app_hash']
        roots[tip]=base64.b64decode(go['state']['result']['response']['last_block_app_hash']).hex()
        require(roots[go['query_height']]==go['app_hash'].lower(),'Go root not in block archive')
        require(base64.b64decode(freeze['app_hash']).hex()==roots[tip],'freeze root differs')
        require(len(go['cases'])==len(requests['cases']) and len(go['cases'])>3000,'truncated case set')
        byname={c['name']:c for c in go['cases']};require(len(byname)==len(go['cases']),'duplicate cases')
        for source,case in zip(requests['cases'],go['cases']):
            for key in ['name','method','request_hex','height','prove']:require(source[key]==case[key],'request changed during capture')
            require({'abci','comet_rpc'}<=set(case['transports'])<=TRANSPORTS,'wrong transport set')
            methods.add(case['method']);totals.update(case['transports'].keys())
        for transport in sorted(TRANSPORTS):
            expected={name:c for name,c in byname.items() if transport in c['transports']};rows=native[transport]
            require(len(rows)==len(expected) and {r['name'] for r in rows}==set(expected),'missing native response')
            for row in rows:
                case=expected[row['name']];answer=case['transports'][transport]
                if transport=='abci':
                    fields=proto_fields(bytes.fromhex(row['response_hex']));value=fields.get(7,[b''])[0].hex();wanted=answer.get('response_hex','')
                elif transport=='cosmos_grpc':
                    require(row['code']==answer.get('code',0),'gRPC status changed');value=row.get('response_hex','');wanted=answer.get('response_hex','')
                elif transport=='comet_rpc':
                    require(row['status']==answer['status'],'Comet HTTP status changed');value=base64.b64decode(row['body']['result']['response'].get('value') or '').hex();wanted=base64.b64decode(answer['body']['result']['response'].get('value') or '').hex()
                else:
                    require(row['status']==answer['status'],'REST status changed');continue
                if value!=wanted:
                    require(case['method'] in SORTED,'unreviewed protobuf normalization')
                    differences.append([scenario,transport,case['name']])
        verdicts=load(cap/'proofs.json');require(len(verdicts)==64,'proof inventory changed')
        expected_proofs={name:c for name,c in byname.items() if c['prove'] and c['transports']['abci']['body']['code']==0 and c['transports']['abci']['body'].get('proofOps') is not None}
        require({v['name'] for v in verdicts}==set(expected_proofs),'unverified Go proof')
        for verdict in verdicts:
            response=expected_proofs[verdict['name']]['transports']['abci']['body'];proofs[verdict['valid']]+=1
            if verdict['valid']:require(verdict['root']==roots[int(response['height'])],'proof root differs from committed root')
            else:require(verdict['error'] and 'root' not in verdict,'rejected proof promoted to valid')
    require(dict(totals)==manifest['counts']['transports'] and len(methods)==manifest['counts']['methods'] and heights==manifest['counts']['matched_heights'],'coverage counts changed')
    require(proofs[True]==616 and proofs[False]==24,'proof outcomes changed')
    require(differences==manifest['normalized_payload_differences'],'normalization inventory changed')
    require(manifest['native_run']['exit_code']==0 and 'test result: ok.' in (HERE/'evidence/native-test.log').read_text(),'native replay did not pass')
    print(f'Verified 10 fresh module campaigns: {dict(totals)}, {heights} matched heights; 616 valid and 24 explicitly rejected proofs')
if __name__=='__main__':verify()
