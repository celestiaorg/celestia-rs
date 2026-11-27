import json, collections, rpc
RAW=rpc.RAW
blk=json.load(open(RAW+"/block_sample_txs.json"))
typ=json.load(open(RAW+"/type_sample_txs.json"))
res={}
def summarize(txs,label):
    per=collections.defaultdict(lambda:{"txs":0,"modes":collections.Counter(),"pubkeys":collections.Counter(),"multisig":0,"amino_types":collections.Counter(),"pairs":collections.Counter(),"wrap":collections.Counter(),"granter":0,"ext":0,"nonempty_memo":0,"timeout_height":0})
    for t in txs:
        if "err" in t: continue
        e=per[str(t["era"])]; e["txs"]+=1
        e["wrap"][t["wrap"] or "plain"]+=1
        if t["granter"]: e["granter"]+=1
        if t["ext"] or t["next"]: e["ext"]+=1
        if t.get("timeout_height"): e["timeout_height"]+=1
        types=set(t["msgs"])|set("%s>%s"%(p,x) for p,x in t["inner"] if p in("authz","gov"))
        for s in t["signers"]:
            for m in s["modes"]: e["modes"][m]+=1
            e["pubkeys"][s["pubkey"] or "none"]+=1
            if "multi" in s["mode"]: e["multisig"]+=1
            for m in set(s["modes"]):
                for ty in types:
                    e["pairs"][ty+"|"+m]+=1
                    if m=="LEGACY_AMINO_JSON": e["amino_types"][ty]+=1
    return {k:{kk:(dict(vv) if isinstance(vv,collections.Counter) else vv) for kk,vv in v.items()} for k,v in sorted(per.items())}
res["block_sample"]=summarize(blk,"block")
res["type_sample"]=summarize(typ,"type")
# dedupe union for pairs
seen=set(); allt=[]
for t in blk+typ:
    if "err" in t: continue
    k=(t["height"],t.get("hash") or t.get("i"))
    allt.append(t)
res["union"]=summarize(allt,"union")
json.dump(res,open(RAW+"/signmode_summary.json","w"),indent=1)
for k in ("block_sample","type_sample"):
    for era,v in res[k].items(): print(k,era,v["txs"],v["modes"],v["multisig"],v["pubkeys"])
