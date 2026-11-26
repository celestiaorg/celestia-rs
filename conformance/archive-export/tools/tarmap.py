import sys, urllib.request, concurrent.futures as cf
url = sys.argv[1]; total = int(sys.argv[2]); N = int(sys.argv[3]); W = int(sys.argv[4])
def rng(a, n):
    r = urllib.request.Request(url, headers={'Range': f'bytes={a}-{a+n-1}'})
    return urllib.request.urlopen(r, timeout=120).read()
def hdr(b):
    if b[257:262] != b'ustar': return None
    try: chk = int(b[148:156].rstrip(b'\0 ').decode() or '0', 8)
    except ValueError: return None
    if chk != sum(b[:148]) + 256 + sum(b[156:512]): return None
    name = b[0:100].rstrip(b'\0').decode(errors='replace'); prefix = b[345:500].rstrip(b'\0').decode(errors='replace')
    sf = b[124:136]
    size = int.from_bytes(sf[1:], 'big') if sf[0] & 0x80 else int(sf.rstrip(b'\0 ').decode() or '0', 8)
    return ((prefix + '/' if prefix else '') + name, size, chr(b[156]) if b[156] else '0')
def probe(k):
    off = (k * total // N) // 512 * 512
    try: b = rng(off, W)
    except Exception as e: return f'{off}\tERR {e}'
    for i in range(0, len(b) - 511, 512):
        h = hdr(b[i:i+512])
        if h: return f'{off+i}\t{h[2]}\t{h[1]}\t{h[0]}'
    return f'{off}\tNONE'
with cf.ThreadPoolExecutor(16) as ex:
    for line in ex.map(probe, range(N)): print(line, flush=True)
