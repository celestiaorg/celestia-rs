"""Reproduce the v11-tagged upgrade fixture using standalone release binaries."""

import argparse
from contextlib import ExitStack, contextmanager
import hashlib
from importlib.util import module_from_spec, spec_from_file_location
import json
from pathlib import Path
import shutil
import socket
import struct

ROOT = Path(__file__).resolve().parents[2]
FIXTURE = ROOT / 'conformance/fixtures/devnet/upgrade'
TAGS = ('v10.4.0', 'v11.0.0-mocha', 'v11.0.0-corto')
SIGNAL_HEIGHT = 99
UPGRADE_HEIGHT = SIGNAL_HEIGHT + 1

# Share the standalone ABCI framing and bounded loopback server from the
# v9 → v10 snapshot probe.
spec = spec_from_file_location('snapshot_probe', Path(__file__).with_name('probe-upgrade-snapshot.py'))
probe = module_from_spec(spec)
spec.loader.exec_module(probe)


def read_fixture():
    requests = []
    hashes = {}
    pending = {}
    height = 0
    data = (FIXTURE / 'abci.rec').read_bytes()
    offset = 0
    while offset < len(data):
        conn, direction, length = struct.unpack_from('<IBI', data, offset)
        body = data[offset + 9:offset + 9 + length]
        offset += 9 + length
        assert len(body) == length and direction in (0, 1)
        kind, payload = next(iter(probe.fields(body).items()))
        if direction == 0 and kind in (5, 8, 11, 16, 17, 20):
            if kind in (16, 17, 20):
                height = probe.fields(payload)[5]
            if kind == 20:
                pending[conn] = height
            requests.append((conn, kind, height, body))
        elif direction == 1 and kind == 21:
            hashes[pending.pop(conn)] = probe.fields(payload)[5].hex()
        elif direction == 1 and kind == 1:
            assert pending.pop(conn) == UPGRADE_HEIGHT
            assert b'UPGRADE "v11" NEEDED' in payload
    assert not pending and set(hashes) == set(range(1, SIGNAL_HEIGHT + 1))
    return requests, hashes


@contextmanager
def application(binary, home, label):
    process, log = probe.start(binary, home, label)
    try:
        with ExitStack() as stack:
            connections = {}

            def call(conn, request):
                if conn not in connections:
                    sock = socket.create_connection(probe.ADDR, timeout=30)
                    connections[conn] = stack.enter_context(sock)
                return probe.call(connections[conn], request)

            yield call
    finally:
        process.terminate()
        try:
            process.wait(timeout=30)
        except probe.subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=10)
        log.close()


def info(call):
    fields = probe.fields(probe.fields(call(999, probe.message(3, b'')))[4])
    return {'app_version': fields[3], 'height': fields[4], 'app_hash': fields[5].hex()}


def record(output, call, conn, request):
    reply = call(conn, request)
    for direction, body in enumerate((request, reply)):
        output.write(struct.pack('<IBI', conn, direction, len(body)) + body)
    return reply


def replay_v10(binary, home, requests, hashes):
    result = probe.run([binary, 'init', 'upgrade-probe', '--chain-id', 'test', '--home', home])
    assert result.returncode == 0, result.stdout
    shutil.copyfile(FIXTURE / 'genesis.json', home / 'config/genesis.json')
    boundary = [(conn, kind, body) for conn, kind, height, body in requests if height > SIGNAL_HEIGHT]
    assert [kind for _, kind, _ in boundary] == [16, 17, 20]
    with application(binary, home, 'v10') as call:
        for conn, kind, height, body in requests:
            if height > SIGNAL_HEIGHT:
                break
            reply = call(conn, body)
            if kind == 20:
                app_hash = probe.fields(probe.fields(reply)[21])[5].hex()
                assert app_hash == hashes[height], f'v10 app hash differs at height {height}'
        signaling_commit = info(call)
        assert signaling_commit == {
            'height': SIGNAL_HEIGHT, 'app_version': 11, 'app_hash': hashes[SIGNAL_HEIGHT],
        }
        *proposal, (conn, _, finalize) = boundary
        for proposal_conn, _, body in proposal:
            call(proposal_conn, body)
        halt = f'UPGRADE "v11" NEEDED at height: {UPGRADE_HEIGHT}'
        try:
            call(conn, finalize)
        except (ConnectionError, EOFError):
            # The socket server can close before flushing the exception.
            pass
        except AssertionError as error:
            assert halt in str(error), error
        else:
            raise AssertionError('v10 unexpectedly handled the v11 plan')
    assert halt in (probe.BASE / 'v10.log').read_text()
    plan = json.loads((home / 'data/upgrade-info.json').read_bytes())
    assert plan['name'] == 'v11' and plan['height'] == UPGRADE_HEIGHT
    return boundary, signaling_commit, plan


def replay_v11(binary, home, label, boundary, signaling_commit, output):
    with output.open('wb') as recording, application(binary, home, label) as call:
        assert info(call) == signaling_commit
        for conn, kind, body in boundary:
            reply = record(recording, call, conn, body)
            if kind == 17:
                assert probe.fields(probe.fields(reply)[18])[1] == 1, 'upgrade proposal rejected'
        app_hash = probe.fields(probe.fields(reply)[21])[5].hex()
        record(recording, call, conn, probe.message(11, b''))
        committed = info(call)
        assert committed == {'height': UPGRADE_HEIGHT, 'app_version': 11, 'app_hash': app_hash}
    with application(binary, home, f'{label}-reopened') as call:
        assert info(call) == committed
    return committed


def sha256(path):
    with path.open('rb') as binary:
        return hashlib.file_digest(binary, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--artifacts', required=True, type=Path, help='directory with <tag>/celestia-appd')
    parser.add_argument('--out', required=True, type=Path, help='new capture directory')
    parser.add_argument('--port', default=28917, type=int)
    args = parser.parse_args()
    if not 0 < args.port <= 65535:
        parser.error('--port must be between 1 and 65535')
    if not __debug__:
        raise RuntimeError('verification requires Python assertions enabled')
    probe.ADDR = ('127.0.0.1', args.port)
    probe.check_port()
    probe.BASE = args.out.resolve()
    probe.BASE.mkdir(parents=True)
    binaries = {tag: (args.artifacts / tag / 'celestia-appd').resolve() for tag in TAGS}
    home = probe.BASE / 'v10-home'
    requests, hashes = read_fixture()
    boundary, signaling_commit, plan = replay_v10(binaries[TAGS[0]], home, requests, hashes)
    summary = {
        'original_fixture_sha256': {name: sha256(FIXTURE / name) for name in ('genesis.json', 'abci.rec')},
        'signaling_commit': signaling_commit,
        'upgrade_info': plan,
        'binary_sha256': {tag: sha256(binary) for tag, binary in binaries.items()},
        'v11_reopened_commits': {},
    }
    recordings = []
    for tag in TAGS[1:]:
        upgraded = probe.BASE / f'{tag}-home'
        shutil.copytree(home, upgraded)
        output = probe.BASE / f'{tag}.rec'
        summary['v11_reopened_commits'][tag] = replay_v11(
            binaries[tag], upgraded, tag, boundary, signaling_commit, output,
        )
        recordings.append(output.read_bytes())
    assert recordings[0] == recordings[1], 'tagged v11 recordings differ'
    capture = probe.BASE / 'abci.rec'
    capture.write_bytes(recordings[0])
    summary['v11_recording_sha256'] = sha256(capture)
    (probe.BASE / 'result.json').write_text(json.dumps(summary, indent=2) + '\n')
    print(f'Both release binaries committed height {UPGRADE_HEIGHT} with app hash '
          f'{summary["v11_reopened_commits"][TAGS[1]]["app_hash"]}')


if __name__ == '__main__':
    main()
