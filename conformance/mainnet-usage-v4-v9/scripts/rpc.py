import json, os, subprocess, time, hashlib, urllib.parse
BASE="https://rpc.celestia.pops.one"
RAW=os.environ.get("INVENTORY_RAW", "raw")+""
CACHE=RAW+"/cache"; os.makedirs(CACHE,exist_ok=True)
NREQ=[0]
def get(path, params=None, timeout=90):
    q = path + ("?"+urllib.parse.urlencode(params) if params else "")
    key=hashlib.sha1(q.encode()).hexdigest()
    f=os.path.join(CACHE,key+".json")
    if os.path.exists(f):
        d=json.load(open(f)); return d["resp"]
    delay=2
    for attempt in range(8):
        NREQ[0]+=1
        try:
            out=subprocess.run(["curl","-s","-m",str(timeout),"-w","\n%{http_code}",BASE+q],capture_output=True,text=True)
            body,_,code=out.stdout.rpartition("\n")
            if code=="200":
                d=json.loads(body)
                if "error" in d and "result" not in d:
                    # deterministic RPC error: cache it
                    json.dump({"q":q,"resp":d},open(f,"w")); return d
                json.dump({"q":q,"resp":d},open(f,"w")); return d
            err=f"http {code} {body[:200]}"
            if "too broad" in body:
                return {"error":"too broad"}
        except Exception as e:
            err=str(e)
        print("retry",q[:120],err,flush=True); time.sleep(delay); delay=min(delay*2,60)
    return {"error":"failed "+err}
def count(query):
    d=get("/tx_search",{"query":'"%s"'%query,"per_page":"1","page":"1","order_by":'"asc"'})
    if "result" not in d: return None,None,d.get("error")
    r=d["result"]; tc=int(r["total_count"]); h=int(r["txs"][0]["height"]) if r["txs"] else None
    return tc,h,None
def search(query,page=1,per_page=20,order="asc"):
    return get("/tx_search",{"query":'"%s"'%query,"per_page":str(per_page),"page":str(page),"order_by":'"%s"'%order})
