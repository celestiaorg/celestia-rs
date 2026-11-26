import zstandard,json,base64,pbdec,glob,os,collections
cov=collections.defaultdict(lambda:{"vectors":0,"ok":0,"files":set(),"pubkeys":set()})
D=os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "fixtures", "antevectors")
for f in sorted(glob.glob(D+"/*.json.zst")):
    j=json.loads(zstandard.ZstdDecompressor().stream_reader(open(f,'rb')).read())
    fn=os.path.basename(f); appv="9" if fn.startswith("v9-") else "10"
    for v in j["vectors"]:
        try: t=pbdec.decode_tx(base64.b64decode(v["tx"]))
        except Exception as e: continue
        modes=set(m for s in t["signers"] for m in s["modes"]) or {"NONE"}
        pks=set(s["pubkey"] for s in t["signers"])
        multi=any("multi" in s["mode"] for s in t["signers"])
        types=set(t["msgs"])|set(x for p,x in t["inner"] if p in("authz","gov"))
        for ty in types:
            for m in modes:
                k=(ty,m,appv); c=cov[k]; c["vectors"]+=1; c["files"].add(fn); c["pubkeys"]|=pks
                if v["expect"]["code"]==0: c["ok"]+=1
                if multi: c.setdefault("multisig",0); c["multisig"]=c.get("multisig",0)+1
out=[{"type":k[0],"mode":k[1],"app":k[2],"vectors":c["vectors"],"ok":c["ok"],"multisig":c.get("multisig",0),"files":sorted(c["files"]),"pubkeys":sorted(x for x in c["pubkeys"] if x)} for k,c in sorted(cov.items())]
json.dump(out,open(os.environ.get("INVENTORY_RAW", "raw")+"/antevectors_coverage.json","w"),indent=1)
for o in out: print(o["app"],o["mode"][:6],o["type"],o["vectors"],o["ok"],o["multisig"])
