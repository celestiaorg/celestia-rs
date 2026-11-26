import re, sys, collections
# Reads `pebble manifest dump` of a MANIFEST and keeps only its first edit
# (the full LSM snapshot written at rotation). Reports per store which IAVL
# key kinds exist and the version range of new-format node keys.
lines = []
started = False
for line in sys.stdin:
    if re.match(r'^\d+/\d+$', line.strip()):
        if started: break
        started = True
    lines.append(line)
def unq(s):
    return bytes.fromhex(s)
pat = re.compile(r'added:\s+L(\d) (\d+):(\d+)<[^>]*>\[(.*?)#\d+,\w+-(.*?)#\d+,\w+\]')
files = []
for l in lines:
    m = pat.search(l)
    if m: files.append((int(m.group(1)), int(m.group(2)), int(m.group(3)), unq(m.group(4)), unq(m.group(5))))
print('files', len(files), 'bytes', sum(f[2] for f in files))
bylevel = collections.Counter(); bybytes = collections.Counter()
for f in files: bylevel[f[0]] += 1; bybytes[f[0]] += f[2]
print('per level', dict(bylevel), {k: round(v/1e12, 3) for k, v in bybytes.items()})
kinds = collections.defaultdict(lambda: [None, None, 0, 0])
for lvl, num, size, lo, hi in files:
    for k in (lo, hi):
        m = re.match(rb's/k:([a-z0-9]+)/(.)(.*)', k, re.S)
        if not m:
            kinds[(b'(meta)', k[:12])][2] += 1; continue
        store, kind, rest = m.group(1), m.group(2), m.group(3)
        e = kinds[(store, kind)]
        e[2] += 1
        if kind == b's' and len(rest) >= 8:
            v = int.from_bytes(rest[:8], 'big')
            e[0] = v if e[0] is None else min(e[0], v)
            e[1] = v if e[1] is None else max(e[1], v)
        if kind == b'r' and len(rest) >= 8:
            v = int.from_bytes(rest[:8], 'big')
            e[0] = v if e[0] is None else min(e[0], v)
            e[1] = v if e[1] is None else max(e[1], v)
for (store, kind), (lo, hi, n, _) in sorted(kinds.items()):
    print(store.decode(errors='replace'), kind.decode(errors='replace') if isinstance(kind, bytes) else kind, 'boundary_keys=%d' % n, 'versions %s..%s' % (lo, hi) if lo is not None else '')
