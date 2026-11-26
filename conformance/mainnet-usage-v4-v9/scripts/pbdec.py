import base64, json
def varint(b,i):
    r=0;s=0
    while True:
        c=b[i];i+=1;r|=(c&0x7f)<<s;s+=7
        if c<0x80: return r,i
def fields(b):
    i=0;out=[]
    while i<len(b):
        k,i=varint(b,i);f=k>>3;w=k&7
        if w==0: v,i=varint(b,i)
        elif w==2:
            l,i=varint(b,i);v=b[i:i+l];i+=l
        elif w==1: v=b[i:i+8];i+=8
        elif w==5: v=b[i:i+4];i+=4
        else: raise ValueError("wire %d"%w)
        out.append((f,w,v))
    return out
def get(b,f): return [v for (ff,w,v) in fields(b) if ff==f]
def get1(b,f,d=None):
    r=get(b,f); return r[0] if r else d
SIGN={0:"UNSPECIFIED",1:"DIRECT",2:"TEXTUAL",3:"DIRECT_AUX",127:"LEGACY_AMINO_JSON",191:"EIP191"}
def unwrap(raw):
    # BlobTx / IndexWrapper
    try:
        fs=fields(raw)
        tid=[v for f,w,v in fs if f==3 and w==2]
        if tid and tid[-1] in (b"BLOB",b"INDX"):
            return unwrap(get1(raw,1))[0], tid[-1].decode()
    except Exception: pass
    return raw,None
def modeinfo(mi):
    s=get1(mi,1)
    if s is not None:
        return {"single":SIGN.get(get1(s,1,0),str(get1(s,1,0)))}
    m=get1(mi,2)
    return {"multi":[modeinfo(x) for x in get(m,2)]}
def flat_modes(m):
    if "single" in m: return [m["single"]]
    r=[]
    for x in m["multi"]: r+=flat_modes(x)
    return r
def any_(b): return (get1(b,1,b"").decode(), get1(b,2,b""))
def inner_msgs(tu,val,depth=0):
    """return list of nested (path,type_url) for interesting wrappers"""
    out=[]
    if depth>4: return out
    try:
        if tu=="/cosmos.authz.v1beta1.MsgExec":
            for a in get(val,2):
                t,v=any_(a); out.append(("authz",t)); out+=[(p,x) for p,x in inner_msgs(t,v,depth+1)]
        elif tu in("/cosmos.gov.v1.MsgSubmitProposal",):
            for a in get(val,1):
                t,v=any_(a); out.append(("gov",t)); out+=inner_msgs(t,v,depth+1)
        elif tu in("/cosmos.gov.v1beta1.MsgSubmitProposal","/cosmos.gov.v1.MsgExecLegacyContent"):
            a=get1(val,1)
            if a: t,v=any_(a); out.append(("content",t))
        elif tu=="/cosmos.authz.v1beta1.MsgGrant":
            g=get1(val,3)
            if g:
                a=get1(g,1)
                if a:
                    t,v=any_(a); out.append(("authorization",t))
                    if t=="/cosmos.authz.v1beta1.GenericAuthorization": out.append(("generic_msg",get1(v,1,b"").decode()))
        elif tu=="/cosmos.feegrant.v1beta1.MsgGrantAllowance":
            a=get1(val,3)
            if a: t,v=any_(a); out.append(("allowance",t))
        elif tu=="/cosmos.vesting.v1beta1.MsgCreateVestingAccount": pass
    except Exception as e:
        out.append(("err",str(e)))
    return out
def decode_tx(raw):
    tx,wrap=unwrap(raw)
    body=get1(tx,1,b""); ai=get1(tx,2,b"")
    msgs=[any_(a) for a in get(body,1)]
    memo=get1(body,2,b"").decode("utf8","replace")
    signers=[]
    for si in get(ai,1):
        pk=get1(si,1); pkt=any_(pk)[0] if pk else None
        mi=get1(si,2); m=modeinfo(mi) if mi else {}
        signers.append({"pubkey":pkt,"mode":m,"modes":flat_modes(m) if m else []})
    fee=get1(ai,2,b"")
    granter=get1(fee,4,b"").decode() if fee else ""
    payer=get1(fee,3,b"").decode() if fee else ""
    ext=len(get(body,1023)); next_=len(get(body,2047))
    inner=[]
    for t,v in msgs: inner+=inner_msgs(t,v)
    return {"wrap":wrap,"msgs":[t for t,_ in msgs],"rawmsgs":msgs,"memo":memo,"signers":signers,"granter":granter,"payer":payer,"ext":ext,"next":next_,"inner":inner,"timeout_height":get1(body,3,0)}
