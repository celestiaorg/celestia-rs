import rpc, pbdec, ibcdec, base64, json, sys
counts=json.load(open(rpc.RAW+"/msg_counts.json"))
ERAS={4:(6680340,6748821),5:(6748822,8662012),6:(8662013,10960599),8:(10960600,11771698),9:(11771699,15116658)}
OUT=rpc.RAW+"/type_sample_txs.json"
out=[]
def dec_resp(d,t,v):
    for tx in d.get("result",{}).get("txs",[]):
        raw=base64.b64decode(tx["tx"]); dec=pbdec.decode_tx(raw)
        details=[]
        for tu,val in dec.pop("rawmsgs"):
            x=ibcdec.msg_detail(tu,val)
            if x: details.append({"type":tu,**x})
        evs=tx["tx_result"].get("events",[])
        out.append({"query_type":t,"era":v,"height":int(tx["height"]),"hash":tx["hash"],"code":tx["tx_result"]["code"],**dec,"details":details,
            "ev_types":sorted(set(e["type"] for e in evs))})
PER=int(sys.argv[1]) if len(sys.argv)>1 else 10
for t,r in counts.items():
    if not r["total"]: continue
    for v,e in r["eras"].items():
        c=e["count"] or 0
        if c==0: continue
        a,b=ERAS[int(v)]
        q="message.action='/%s' AND tx.height>=%d AND tx.height<=%d"%(t,a,b)
        if e.get("estimated"):
            for wa in (a, (a+b)//2, b-3000):
                q="message.action='/%s' AND tx.height>=%d AND tx.height<=%d"%(t,wa,wa+3000)
                d=rpc.search(q,1,PER,"asc")
                if "result" in d: dec_resp(d,t,v)
            continue
        d=rpc.search(q,1,PER,"asc")
        if "result" in d: dec_resp(d,t,v)
        if c>PER and not e.get("estimated"):
            d=rpc.search(q,1,PER,"desc")
            if "result" in d: dec_resp(d,t,v)
            if c>4*PER:
                mid=(c//PER)//2+1
                d=rpc.search(q,mid,PER,"asc")
                if "result" in d: dec_resp(d,t,v)
    print(t,"reqs",rpc.NREQ[0],len(out),flush=True)
json.dump(out,open(OUT,"w"),indent=0)
