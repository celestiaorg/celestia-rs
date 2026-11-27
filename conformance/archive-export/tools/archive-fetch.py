"""Extract selected members of a remote tar with parallel HTTP range requests.
usage: archive-fetch.py URL INDEX DEST PREFIX THREADS
INDEX lines: offset<TAB>type<TAB>size<TAB>name (tar header offsets, from tarindex.py).
Members whose name starts with PREFIX are written under DEST (PREFIX stripped).
Resumable: a member whose file exists with the right size is skipped."""
import sys, os, time, threading, http.client, urllib.parse, concurrent.futures as cf
url, index, dest, prefix, threads = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4], int(sys.argv[5])
skip = set(os.environ.get('SKIP', '').split(',')) - {''}
u = urllib.parse.urlparse(url)
os.makedirs(dest, exist_ok=True)
members = []
for line in open(index):
    off, typ, size, name = line.rstrip('\n').split('\t', 3)
    if typ != '0' or not name.startswith(prefix): continue
    rel = name[len(prefix):]
    if not rel or '/' in rel or rel in skip or rel.startswith('.'): continue
    members.append((int(off), int(size), rel))
members.sort()
todo = [m for m in members if not (os.path.exists(os.path.join(dest, m[2])) and os.path.getsize(os.path.join(dest, m[2])) == m[1])]
total = sum(m[1] for m in members); left = sum(m[1] for m in todo)
print(f'{len(members)} members, {total/1e12:.3f} TB; to fetch {len(todo)} members, {left/1e12:.3f} TB', flush=True)
# Batch contiguous members into ranges of up to 64 MiB.
batches, cur = [], []
for m in todo:
    if cur and (m[0] != cur[-1][0] + 512 + (cur[-1][1] + 511) // 512 * 512 or m[0] - cur[0][0] > 64 << 20):
        batches.append(cur); cur = []
    cur.append(m)
if cur: batches.append(cur)
local = threading.local(); done = [0]; lock = threading.Lock()
def get(a, b):
    for attempt in range(10):
        try:
            if not hasattr(local, 'c'): local.c = http.client.HTTPSConnection(u.netloc, timeout=300)
            local.c.request('GET', u.path, headers={'Range': f'bytes={a}-{b-1}'})
            r = local.c.getresponse(); d = r.read()
            if r.status == 206 and len(d) == b - a: return d
        except Exception:
            pass
        local.__dict__.pop('c', None); time.sleep(min(60, 2 ** attempt))
    raise RuntimeError(f'range {a}-{b} failed')
def run(batch):
    a = batch[0][0]; last = batch[-1]; b = last[0] + 512 + last[1]
    d = get(a, b)
    for off, size, rel in batch:
        p = os.path.join(dest, rel)
        with open(p + '.part', 'wb') as f: f.write(d[off - a + 512: off - a + 512 + size])
        os.replace(p + '.part', p)
    with lock: done[0] += b - a
t0 = time.time()
def report():
    while True:
        time.sleep(60)
        with lock: n = done[0]
        el = time.time() - t0
        print(f'{time.strftime("%H:%M:%S")} fetched {n/1e12:.3f}/{left/1e12:.3f} TB, {n/el/1e6:.0f} MB/s, eta {((left - n) / (n / el) / 3600) if n else 0:.1f} h', flush=True)
threading.Thread(target=report, daemon=True).start()
with cf.ThreadPoolExecutor(threads) as ex:
    for _ in ex.map(run, batches): pass
print('done', flush=True)
