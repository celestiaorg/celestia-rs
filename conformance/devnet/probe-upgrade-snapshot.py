"""Pinned-Go-only diagnostic. Replay unchanged ABCI frames through H/H+1.
Homes, logs and public snapshot captures stay under --out (default target/).
No consensus node or public network is used; all ABCI requests are unmodified.
"""
import argparse
import contextlib
import hashlib
import io
import json
import pathlib
import shutil
import socket
import struct
import subprocess
import time
ROOT = pathlib.Path(__file__).resolve().parents[2]
BASE = ROOT / 'target/go-upgrade-snapshot'
FIXTURE = ROOT / 'conformance/fixtures/devnet/v9-upgrade-replay'
V9 = ROOT / 'target/devnet-bin/v9/celestia-appd'
V10 = ROOT / 'target/devnet-bin/celestia-appd'
ADDR = ('127.0.0.1', 28761)

def varint(n):
    out = bytearray()
    while n > 127:
        out.append(n & 127 | 128)
        n >>= 7
    return bytes(out + bytes([n]))

def read_varint(read):
    result = 0
    for i in range(10):
        b = read(1)
        if not b:
            raise EOFError('truncated varint')
        result |= (b[0] & 127) << 7 * i
        if b[0] < 128:
            return result
    raise ValueError('overlong varint')

def fields(data):
    stream = io.BytesIO(data)
    result = {}
    while stream.tell() < len(data):
        key = read_varint(stream.read)
        wire = key & 7
        if wire == 0:
            value = read_varint(stream.read)
        elif wire == 2:
            length = read_varint(stream.read)
            value = stream.read(length)
            assert len(value) == length, 'truncated protobuf field'
        elif wire in (1, 5):
            value = stream.read(8 if wire == 1 else 4)
        else:
            raise ValueError(f'unsupported wire type {wire}')
        result[key >> 3] = value
    return result

def receive(conn):
    length = read_varint(conn.recv)
    assert length <= 64 << 20, 'oversized ABCI frame'
    out = bytearray()
    while len(out) < length:
        chunk = conn.recv(length - len(out))
        if not chunk:
            raise EOFError('truncated frame')
        out.extend(chunk)
    return bytes(out)

def call(conn, request):
    flush = b'\x12\x00'
    conn.sendall(varint(len(request)) + request)
    if request != flush:
        conn.sendall(varint(len(flush)) + flush)
    response = receive(conn)
    if request != flush:
        assert 3 in fields(receive(conn)), 'missing flush response'
    assert 1 not in fields(response), f'ABCI exception: {response!r}'
    return response

def records():
    data = (FIXTURE / 'abci.rec').read_bytes()
    offset = 0
    out = []
    while offset < len(data):
        conn, direction, length = struct.unpack_from('<IBI', data, offset)
        body = data[offset + 9:offset + 9 + length]
        assert len(body) == length and direction in (0, 1), 'invalid recording frame'
        offset += 9 + length
        if direction == 0:
            out.append((conn, body))
    assert offset == len(data)
    return out

def run(args, **kwargs):
    kwargs.setdefault('timeout', 60)
    return subprocess.run([str(x) for x in args], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, **kwargs)

def check_port():
    # Never send fixture traffic to a pre-existing local ABCI application.
    with socket.socket() as probe:
        probe.bind(ADDR)


def start(binary, home, label):
    check_port()
    log = (BASE / f'{label}.log').open('wb')
    proc = subprocess.Popen([str(binary), 'start', '--force-no-bbr', '--home', str(home), '--with-comet=false', '--address', f'tcp://{ADDR[0]}:{ADDR[1]}', '--grpc.enable=false'], stdout=log, stderr=subprocess.STDOUT)
    for _ in range(100):
        if proc.poll() is not None:
            log.close()
            raise RuntimeError((BASE / f'{label}.log').read_text())
        if 'Waiting for new connection... module=abci-server' not in (BASE / f'{label}.log').read_text():
            time.sleep(0.1)
            continue
        try:
            socket.create_connection(ADDR, timeout=0.2).close()
            return (proc, log)
        except OSError:
            time.sleep(0.1)
    proc.terminate()
    proc.wait(timeout=20)
    log.close()
    raise TimeoutError('ABCI start')

def replay(binary, home, requests, first, stop, label, expected):
    proc, log = start(binary, home, label)
    connections = {}
    height = 0
    try:
        for i in range(first, len(requests)):
            cid, request = requests[i]
            conn = connections.get(cid)
            if conn is None:
                conn = socket.create_connection(ADDR, timeout=15)
                connections[cid] = conn
            parsed = fields(request)
            if 20 in parsed:
                height = fields(parsed[20])[5]
            response = call(conn, request)
            if 20 in parsed:
                ours = fields(fields(response)[21]).get(5, b'')
                assert ours == expected[height], f'Go app-hash mismatch at {height}'
            if 11 in parsed and height == stop:
                print(f'{label}: committed {height} with original Go app hash', flush=True)
                return i + 1
        raise ValueError('missing stop height')
    finally:
        for conn in connections.values():
            conn.close()
        proc.terminate()
        proc.wait(timeout=30)
        log.close()

def expected_hashes():
    data = (FIXTURE / 'abci.rec').read_bytes()
    offset = 0
    result = {}
    pending = {}
    upgrades = []
    previous_version = None
    while offset < len(data):
        cid, dr, n = struct.unpack_from('<IBI', data, offset)
        body = data[offset + 9:offset + 9 + n]
        offset += 9 + n
        parsed = fields(body)
        if dr == 0 and 20 in parsed:
            pending[cid] = fields(parsed[20])[5]
        if dr == 1 and 21 in parsed:
            height = pending.pop(cid)
            response = fields(parsed[21])
            result[height] = response.get(5, b'')
            assert len(result[height]) == 32, 'missing recorded app hash'
            params = fields(response.get(4, b''))
            if 4 in params:
                version = fields(params[4]).get(1, 0)
                if previous_version == 9 and version == 10:
                    upgrades.append(height)
                previous_version = version
    assert not pending, 'unmatched recorded FinalizeBlock'
    assert len(upgrades) == 1, 'expected one v9-to-v10 transition'
    return (result, upgrades[0])

def main():
    global BASE, ADDR
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=pathlib.Path, default=BASE, help='new directory for homes, provenance and logs')
    parser.add_argument('--port', type=int, default=ADDR[1])
    args = parser.parse_args()
    if not 0 < args.port <= 65535:
        parser.error('--port must be between 1 and 65535')
    BASE = args.out.resolve()
    ADDR = ('127.0.0.1', args.port)
    try:
        check_port()
    except OSError as error:
        parser.error(f'probe port is already in use: {ADDR}: {error}')
    if not __debug__:
        raise RuntimeError('verification requires Python assertions enabled')
    provenance = {}
    for binary, module in [(V9, 'github.com/celestiaorg/celestia-app/v9@v9.0.8'), (V10, 'github.com/celestiaorg/celestia-app/v10@v10.5.0')]:
        assert pathlib.Path(str(binary) + '.module').read_text().strip() == module, f'wrong source receipt: {binary}'
        info = run(['go', 'version', '-m', binary])
        assert info.returncode == 0 and info.stdout.splitlines()[0].endswith('go1.26.9'), 'requires pinned Go1.26.9 binaries'
        provenance[str(binary)] = {'module': module, 'sha256': hashlib.file_digest(binary.open('rb'), 'sha256').hexdigest(), 'build_info': info.stdout}
    BASE.mkdir(parents=True, exist_ok=False)
    home = BASE / 'at-h'
    init = run([V9, 'init', 'snapshot-probe', '--chain-id', 'test', '--home', home])
    assert init.returncode == 0, init.stdout
    shutil.copyfile(FIXTURE / 'genesis.json', home / 'config/genesis.json')
    cfg = home / 'config/app.toml'
    cfg.write_text(cfg.read_text().replace('pruning = "default"', 'pruning = "nothing"'))
    req = records()
    expected, upgrade = expected_hashes()
    next_index = replay(V9, home, req, 0, upgrade, 'v9-through-h', expected)
    after = BASE / 'at-h-plus-one'
    shutil.copytree(home, after)
    outcomes = {'scenario': FIXTURE.name, 'upgrade_height': upgrade, 'binaries': provenance, 'fixture_sha256': {name: hashlib.sha256((FIXTURE / name).read_bytes()).hexdigest() for name in ['genesis.json', 'abci.rec']}, 'exports': {}}
    # v9 can export H before a v10 process opens the database.
    v9_home = BASE / 'at-h-v9-export'
    shutil.copytree(home, v9_home)
    v9_export = run([V9, 'snapshots', 'export', '--home', v9_home, '--height', upgrade])
    (BASE / 'export-h-v9.log').write_text(v9_export.stdout)
    assert v9_export.returncode == 0 and 'Snapshot created' in v9_export.stdout
    outcomes['v9_export_h'] = {'exit_code': v9_export.returncode, 'output': v9_export.stdout}
    for height, destination in [(upgrade, home), (upgrade + 1, after)]:
        if height == upgrade + 1:
            replay(V10, destination, req, next_index, height, 'v10-through-h-plus-one', expected)
        result = run([V10, 'snapshots', 'export', '--home', destination, '--height', height])
        (BASE / f'export-{height}.log').write_text(result.stdout)
        outcomes['exports'][str(height)] = {
            'exit_code': result.returncode, 'output': result.stdout,
            'app_hash': expected[height].hex(),
        }
        print(f'export {height}: exit {result.returncode}', flush=True)
    outcomes['restores'] = {}
    for height, source in [(upgrade, v9_home), (upgrade + 1, after)]:
        outcomes['restores'][str(height)] = restore_probe(source, height, expected[height], upgrade)
    (BASE / 'outcomes.json').write_text(json.dumps(outcomes, indent=2) + '\n')
    before = outcomes['exports'][str(upgrade)]
    after = outcomes['exports'][str(upgrade + 1)]
    assert before['exit_code'] != 0 and 'version does not exist' in before['output'] and 'store=fibre' in before['output'], 'unexpected H export outcome'
    assert after['exit_code'] == 0 and 'Snapshot created' in after['output'], 'H+1 snapshot export failed'
    assert outcomes['restores'][str(upgrade)]['apply_result'] == 2
    assert outcomes['restores'][str(upgrade + 1)]['apply_result'] == 1


# Field numbers below are from pinned core's proto/tendermint/abci/types.proto.
def message(number, value):
    return varint((number << 3) | 2) + varint(len(value)) + value


def integer(number, value):
    return varint(number << 3) + varint(value)


@contextlib.contextmanager
def application(binary, home, label):
    proc, log = start(binary, home, label)
    conn = None
    try:
        conn = socket.create_connection(ADDR, timeout=15)
        yield conn
    finally:
        if conn is not None:
            conn.close()
        proc.terminate()
        try:
            proc.wait(timeout=30)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=10)
        log.close()


def restore_probe(source, height, app_hash, upgrade):
    capture = BASE / 'snapshots' / str(height)
    capture.mkdir(parents=True)
    with application(V10, source, f'serve-{height}') as conn:
        listed = fields(fields(call(conn, message(12, b'')))[13])
        snapshot = listed[1]
        header = fields(snapshot)
        assert header[1] == height and header[2] == 3 and header[3] == 1
        (capture / 'snapshot.proto').write_bytes(snapshot)
        chunk_request = integer(1, height) + integer(2, 3) + integer(3, 0)
        chunk = fields(fields(call(conn, message(14, chunk_request)))[15])[1]
        (capture / 'chunk-0').write_bytes(chunk)
    destination = BASE / f'restore-{height}'
    initialized = run([V10, 'init', 'restore-probe', '--chain-id', 'test', '--home', destination])
    assert initialized.returncode == 0, initialized.stdout
    shutil.copyfile(FIXTURE / 'genesis.json', destination / 'config/genesis.json')
    with application(V10, destination, f'restore-{height}') as conn:
        offer = message(1, snapshot) + message(2, app_hash) + integer(3, 10)
        offered = fields(fields(call(conn, message(13, offer)))[14]).get(1, 0)
        assert offered == 1
        apply = integer(1, 0) + message(2, chunk) + message(3, b'local-probe')
        applied = fields(fields(call(conn, message(15, apply)))[16]).get(1, 0)
        info = fields(fields(call(conn, message(3, b'')))[4])
        info = {'height': info.get(4, 0), 'app_version': info.get(3, 0),
                'app_hash': info.get(5, b'').hex()}
    if height == upgrade:
        assert applied == 2 and info['height'] == 0
        error_log = (BASE / f'restore-{height}.log').read_text()
        assert 'version of store valaddr mismatch' in error_log and 'StoreUpgrades' in error_log
    else:
        assert applied == 1 and info['height'] == height and info['app_version'] == 10
        assert info['app_hash'] == app_hash.hex()
    result = {'height': height, 'producer': 'v9.0.8' if height == upgrade else 'v10.5.0',
              'receiver': 'v10.5.0', 'core': 'v0.42.3', 'compiler': 'go1.26.9',
              'app_version': 10, 'app_hash': app_hash.hex(), 'offer_result': offered,
              'apply_result': applied, 'restored_info': info,
              'snapshot_sha256': hashlib.sha256(snapshot).hexdigest(),
              'chunk_sha256': hashlib.sha256(chunk).hexdigest()}
    (capture / 'oracle.json').write_text(json.dumps(result, indent=2) + '\n')
    print(f'restore {height}: offer={offered} apply={applied} info={info}', flush=True)
    return result


if __name__ == '__main__':
    main()
