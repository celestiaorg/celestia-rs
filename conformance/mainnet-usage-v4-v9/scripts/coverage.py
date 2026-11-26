import json,collections,zstandard,glob,os,base64,pbdec
RAW="../audit/raw"
# antevectors pairs, keeping authz-inner separate
av=collections.defaultdict(lambda:{"n":0,"ok":0,"multi":0,"apps":set()})
for f in sorted(glob.glob("/home/user/celestia-rs/conformance/fixtures/antevectors/*.json.zst")):
    fn=os.path.basename(f); app="9" if fn.startswith("v9-") else "10"
    j=json.loads(zstandard.ZstdDecompressor().stream_reader(open(f,'rb')).read())
    for v in j["vectors"]:
        try: t=pbdec.decode_tx(base64.b64decode(v["tx"]))
        except Exception: continue
        types=set(t["msgs"])|set("%s>%s"%(p,q) for p,q in t["inner"] if p in("authz","gov"))
        for s in t["signers"]:
            for m in set(s["modes"]):
                for ty in types:
                    a=av[(ty,m)]; a["n"]+=1; a["apps"].add(app)
                    if v["expect"]["code"]==0: a["ok"]+=1
                    if "multi" in s["mode"]: a["multi"]+=1
corpus=set((x["type"],x["mode"]) for x in json.load(open(RAW+"/mainnet_corpus_v6_coverage.json")))
mn=collections.defaultdict(lambda:{"eras":set(),"n":0,"multi":0,"multi_eras":set()})
for x in json.load(open(RAW+"/block_sample_txs.json"))+json.load(open(RAW+"/type_sample_txs.json")):
    types=set(x["msgs"])|set("%s>%s"%(p,q) for p,q in x["inner"] if p in("authz","gov"))
    for s in x["signers"]:
        for m in set(s["modes"]):
            for ty in types:
                r=mn[(ty,m)]; r["eras"].add(int(x["era"])); r["n"]+=1
                if "multi" in s["mode"]: r["multi"]+=1; r["multi_eras"].add(int(x["era"]))
rows=[]
for (ty,m),r in sorted(mn.items()):
    a=av.get((ty,m))
    rows.append({"type":ty,"mode":m,"mainnet_eras":sorted(r["eras"]),"mainnet_sample_n":r["n"],"mainnet_multisig_n":r["multi"],
        "antevectors_n":a["n"] if a else 0,"antevectors_ok":a["ok"] if a else 0,"antevectors_multisig_n":a["multi"] if a else 0,
        "antevectors_apps":sorted(a["apps"]) if a else [],"in_v6_mainnet_corpus":(ty,m) in corpus})
json.dump(rows,open(RAW+"/coverage_pairs.json","w"),indent=1)
unc=[r for r in rows if r["antevectors_n"]==0]
okless=[r for r in rows if r["antevectors_n"]>0 and r["antevectors_ok"]==0]
msunc=[r for r in rows if r["mainnet_multisig_n"]>0 and r["antevectors_multisig_n"]==0]
print("pairs",len(rows),"uncovered",len(unc),"covered-but-no-accepted-vector",len(okless),"multisig-uncovered",len(msunc))
for r in unc: print("UNC",r["type"],r["mode"],r["mainnet_eras"],r["mainnet_sample_n"],"corpus" if r["in_v6_mainnet_corpus"] else "")
for r in okless: print("NOOK",r["type"],r["mode"],r["antevectors_n"])
for r in msunc: print("MSUNC",r["type"],r["mode"],r["mainnet_multisig_n"])
