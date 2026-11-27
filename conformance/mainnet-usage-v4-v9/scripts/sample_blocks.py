import rpc, pbdec, base64, json, sys
ERAS=[(4,6680340,6748821),(5,6748822,8662012),(6,8662013,10960599),(8,10960600,11771698),(9,11771699,15116658)]
N=int(sys.argv[1]) if len(sys.argv)>1 else 80
out=[]
for v,a,b in ERAS:
    step=(b-a)//N
    for k in range(N):
        h=a+k*step+(k*7919)%max(step,1)//3  # jitter
        d=rpc.get("/block",{"height":str(h)})
        if "result" not in d: continue
        for i,t in enumerate(d["result"]["block"]["data"]["txs"] or []):
            try: dec=pbdec.decode_tx(base64.b64decode(t))
            except Exception as e:
                out.append({"era":v,"height":h,"i":i,"err":str(e)}); continue
            dec.pop("rawmsgs")
            out.append({"era":v,"height":h,"i":i,**dec})
    print("era",v,"done reqs",rpc.NREQ[0],flush=True)
json.dump(out,open(rpc.RAW+"/block_sample_txs.json","w"),indent=0)
print(len(out))
