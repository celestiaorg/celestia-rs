"""Index a remote tar by walking headers between known header offsets in parallel.
usage: tarindex.py URL TARMAP_OUT START END THREADS > index.tsv
Prints offset<TAB>type<TAB>size<TAB>name for every header in [START, END)."""
import sys, urllib.request, concurrent.futures as cf, http.client, urllib.parse
url, mapf, start, end, threads = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5])
u = urllib.parse.urlparse(url)
import threading
local = threading.local()
def conn():
    if not hasattr(local, 'c'):
        local.c = http.client.HTTPSConnection(u.netloc, timeout=120)
    return local.c
def rng(a, n):
    for attempt in range(6):
        try:
            c = conn(); c.request('GET', u.path, headers={'Range': f'bytes={a}-{a+n-1}'})
            r = c.getresponse(); d = r.read()
            if r.status == 206 and len(d) == n: return d
        except Exception:
            pass
        local.__dict__.pop('c', None)
    raise RuntimeError(f'range {a} failed')
def parse(b):
    if b[257:262] != b'ustar': return None
    name = b[0:100].rstrip(b'\0').decode(errors='replace'); prefix = b[345:500].rstrip(b'\0').decode(errors='replace')
    sf = b[124:136]
    size = int.from_bytes(sf[1:], 'big') if sf[0] & 0x80 else int(sf.rstrip(b'\0 ').decode() or '0', 8)
    return ((prefix + '/' if prefix else '') + name, size, chr(b[156]) if b[156] else '0')
offs = sorted({int(l.split('\t')[0]) for l in open(mapf) if l.split('\t')[1].strip() in ('0', '5', 'L')})
offs = [o for o in offs if start <= o < end]
segs = list(zip([start] + offs, offs + [end]))
segs = [(a, b) for a, b in segs if a < b]
def walk(seg):
    a, b = seg; out = []; off = a; longname = None
    while off < b:
        h = rng(off, 512); p = parse(h)
        if p is None: out.append(f'{off}\tBAD\t0\t?'); break
        name, size, typ = p
        if typ == 'L':
            longname = rng(off + 512, size).rstrip(b'\0').decode(); off += 512 + (size + 511) // 512 * 512; continue
        out.append(f'{off}\t{typ}\t{size}\t{longname or name}'); longname = None
        off += 512 + (size + 511) // 512 * 512
    return out
with cf.ThreadPoolExecutor(threads) as ex:
    for lines in ex.map(walk, segs):
        sys.stdout.write('\n'.join(lines) + '\n'); sys.stdout.flush()
