import pbdec, json, base64
from pbdec import get1, get, any_
def packet(p):
    return {"seq":get1(p,1,0),"src_port":get1(p,2,b"").decode(),"src_chan":get1(p,3,b"").decode(),
            "dst_port":get1(p,4,b"").decode(),"dst_chan":get1(p,5,b"").decode(),"data":get1(p,6,b"")}
def packet_data(pk):
    d=pk["data"]; r={"ports":f'{pk["src_port"]}/{pk["src_chan"]}->{pk["dst_port"]}/{pk["dst_chan"]}'}
    try:
        j=json.loads(d)
        if "denom" in j:
            r["kind"]="transfer"; r["denom"]=j.get("denom"); memo=j.get("memo","") or ""
            r["memo_forward"]="forward" in memo; r["memo_wasm"]="wasm" in memo; r["memo_head"]=memo[:200]
        elif "type" in j:
            r["kind"]="ica"; r["ica_type"]=j.get("type")
            data=j.get("data")
            try:
                raw=base64.b64decode(data); r["ica_msgs"]=[any_(a)[0] for a in get(raw,1)]
            except Exception:
                try:
                    jj=json.loads(base64.b64decode(data)); r["ica_msgs"]=[m.get("@type") for m in jj.get("messages",[])]
                except Exception as e: r["ica_err"]=str(e)
        else: r["kind"]="json-other"; r["keys"]=list(j)[:8]
    except Exception:
        r["kind"]="nonjson"; r["len"]=len(d)
    return r
def msg_detail(tu,val):
    try:
        if tu=="/ibc.core.channel.v1.MsgRecvPacket":
            return packet_data(packet(get1(val,1,b"")))
        if tu in("/ibc.core.channel.v1.MsgAcknowledgement","/ibc.core.channel.v1.MsgTimeout","/ibc.core.channel.v1.MsgTimeoutOnClose"):
            pk=packet(get1(val,1,b"")); r=packet_data(pk); return r
        if tu=="/ibc.core.client.v1.MsgUpdateClient":
            cid=get1(val,1,b"").decode(); t,v=any_(get1(val,2,b""))
            r={"client_id":cid,"client_msg":t}
            if t=="/ibc.lightclients.tendermint.v1.Header":
                sh=get1(v,1,b""); hd=get1(sh,1,b""); r["chain_id"]=get1(hd,2,b"").decode()
            return r
        if tu=="/ibc.core.client.v1.MsgCreateClient":
            t,v=any_(get1(val,1,b"")); r={"client_state":t}
            if "tendermint" in t: r["chain_id"]=get1(v,1,b"").decode()
            return r
        if tu=="/ibc.applications.transfer.v1.MsgTransfer":
            memo=get1(val,8,b"").decode("utf8","replace")
            return {"src":get1(val,1,b"").decode()+"/"+get1(val,2,b"").decode(),"memo_forward":"forward" in memo,"memo_head":memo[:120]}
        if tu=="/cosmos.gov.v1.MsgSubmitProposal":
            return {"title":get1(val,5,b"").decode("utf8","replace")[:120],"expedited":get1(val,7,0)}
    except Exception as e: return {"err":str(e)}
    return None
