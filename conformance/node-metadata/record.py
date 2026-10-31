#!/usr/bin/env python3
"""Capture consensus metadata from an explicitly selected isolated Go node.

The artifact preserves complete wire bodies. The regression checks field sets
and stable scalar types, not equality of asynchronous consensus state.
"""
import argparse
import datetime
import hashlib
import ipaddress
import json
import pathlib
import subprocess
import urllib.parse
import urllib.request


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rpc', required=True)
    parser.add_argument('--binary', required=True, type=pathlib.Path)
    parser.add_argument('--genesis', required=True, type=pathlib.Path)
    parser.add_argument('--output', required=True, type=pathlib.Path)
    args = parser.parse_args()
    url = urllib.parse.urlparse(args.rpc)
    if url.scheme != 'http' or not ipaddress.ip_address(url.hostname).is_loopback:
        parser.error('--rpc must select an isolated HTTP loopback listener')
    build = subprocess.check_output(['go', 'version', '-m', str(args.binary)], text=True)
    if 'github.com/celestiaorg/celestia-core\tv0.42.3' not in build:
        parser.error('binary does not report the pinned celestia-core dependency')
    if 'github.com/celestiaorg/cosmos-sdk\tv0.52.12' not in build:
        parser.error('binary does not report the pinned Cosmos SDK replacement')
    if '\tbuild\t-tags=ledger\n' not in build or 'version.Version=10.5.0' not in build:
        parser.error('binary lacks the selected standalone ledger/release identity flags')
    module = 'github.com/celestiaorg/celestia-app/v10@v10.5.0'
    stamp = pathlib.Path(str(args.binary) + '.module')
    if stamp.read_text().strip() != module:
        parser.error('binary lacks the successful pinned-source build stamp')
    source = json.loads(subprocess.check_output(['go', 'mod', 'download', '-json', module], text=True))
    commit = source['Origin']['Hash']
    if commit != '6013e758d5dceb75827ad191c8d739d7c43cfa04':
        parser.error('unexpected application release commit')
    digest = hashlib.sha256(args.binary.read_bytes()).hexdigest()
    recipe = json.loads(pathlib.Path(str(args.binary) + '.build.json').read_text())
    if (recipe['binary_sha256'] != digest or recipe['source_commit'] != commit
            or recipe['module'] != module or recipe['build_tags'] != ['ledger']):
        parser.error('binary does not match its pinned successful-build recipe')
    out = {
        'scope': 'Current-release source, actual local Go transport. Consensus field sets and scalar encodings only; no matched-state equality or official release binary claim.',
        'observed_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
        'provenance': {
            'application_module': module,
            'application_commit': commit,
            'application_sum': source['Sum'],
            'go_version': subprocess.check_output(['go', 'version'], text=True).strip(),
            'binary_sha256': digest,
            'genesis_sha256': hashlib.sha256(args.genesis.read_bytes()).hexdigest(),
            'recorder_sha256': hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
            'go_build_info': build,
            'build_recipe': recipe,
        },
        'responses': {},
    }
    for route, suffix in [('status', 'status'), ('net_info', 'net_info'),
                          ('abci_info', 'abci_info'),
                          ('app_version', 'abci_query?path=%22/app/version%22'),
                          ('consensus_state', 'consensus_state'),
                          ('dump_consensus_state', 'dump_consensus_state')]:
        request = args.rpc.rstrip('/') + '/' + suffix
        with urllib.request.urlopen(request, timeout=20) as response:
            body = response.read(2 * 1024 * 1024 + 1)
            if len(body) > 2 * 1024 * 1024:
                raise ValueError('response exceeds bounded capture size')
            raw = body.decode()
            decoded = json.loads(raw)
            if response.status != 200 or 'error' in decoded or 'result' not in decoded:
                raise ValueError(f'{route}: failed capture: {raw}')
            out['responses'][route] = {'request': request, 'status': response.status,
                                       'headers': list(response.headers.items()), 'body': raw}
    info = json.loads(out['responses']['abci_info']['body'])['result']['response']
    if info.get('data') != 'celestia-app' or info.get('version') != '10.5.0':
        raise ValueError('running node does not report the selected release identity')
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(out, indent=2) + '\n')


if __name__ == '__main__':
    main()
