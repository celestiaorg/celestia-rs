import rpc, json, sys
ERAS=[(4,6680340,6748821),(5,6748822,8662012),(6,8662013,10960599),(8,10960600,11771698),(9,11771699,15116658)]
ALL=(6680340,15116658)
OUT=rpc.RAW+"/msg_counts.json"
try: res=json.load(open(OUT))
except: res={}
def q(t,a,b): return "message.action='/%s' AND tx.height>=%d AND tx.height<=%d"%(t,a,b)
def cnt(t,a,b):
    """returns (count, first_height, estimated_flag)"""
    tc,h,err=rpc.count(q(t,a,b))
    if err is None: return tc,h,False
    if err!="too broad": raise SystemExit(f"error {t} {a} {b} {err}")
    # estimate from K evenly spaced windows of W blocks
    K=4; W=3000; tot=0; first=None
    for k in range(K):
        wa=a+k*((b-a-W)//(K-1))
        c,hh,e=rpc.count(q(t,wa,wa+W-1))
        if e: 
            c,hh,e=rpc.count(q(t,wa,wa+299)); c=(c or 0)*10
        tot+=c or 0
        if first is None and hh: first=hh
    return int(tot*(b-a+1)/(K*W)),first,True
for t in [l.strip() for l in open("cands.txt") if l.strip()]:
    if t in res and "eras" in res[t]: continue
    r=res.get(t,{})
    tc,h,err=rpc.count(q(t,*ALL))
    if err is None and tc==0:
        res[t]={"total":0,"eras":{}}
    else:
        eras={}
        for v,a,b in ERAS:
            c,fh,est=cnt(t,a,b)
            eras[str(v)]={"count":c,"first":fh,"estimated":est}
        res[t]={"total":(tc if err is None else sum((e["count"] or 0) for e in eras.values())),"total_exact":err is None,"eras":eras}
    json.dump(res,open(OUT,"w"),indent=1)
    print(t,json.dumps(res[t]),"reqs",rpc.NREQ[0],flush=True)
