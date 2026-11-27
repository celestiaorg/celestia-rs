import rpc, json
ERAS=[(4,6680340,6748821),(5,6748822,8662012),(6,8662013,10960599),(8,10960600,11771698),(9,11771699,15116658)]
res={}
def era_counts(name,cond):
    r={}
    tc,h,err=rpc.count(f"{cond} AND tx.height>=6680340 AND tx.height<=15116658")
    if err is None and tc==0:
        res[name]={"total":0}; print(name,0,flush=True); return
    for v,a,b in ERAS:
        tc,h,err=rpc.count(f"{cond} AND tx.height>={a} AND tx.height<={b}")
        r[str(v)]={"count":tc,"first":h,"err":err}
    res[name]=r; print(name,r,flush=True)
for ct in ["07-tendermint","06-solomachine","08-wasm","09-localhost"]:
    era_counts("update_client:"+ct, f"update_client.client_type='{ct}'")
for ct in ["07-tendermint","06-solomachine","08-wasm"]:
    era_counts("create_client:"+ct, f"create_client.client_type='{ct}'")
era_counts("recv_packet:icahost","recv_packet.packet_dst_port='icahost'")
era_counts("recv_packet:transfer","recv_packet.packet_dst_port='transfer'")
era_counts("send_packet:transfer","send_packet.packet_src_port='transfer'")
era_counts("pfm_forward(recv+send transfer)","message.action='/ibc.core.channel.v1.MsgRecvPacket' AND send_packet.packet_src_port='transfer'")
era_counts("send_packet:icacontroller-any","channel_open_init.port_id='icahost'")
era_counts("channel_open_try:icahost","channel_open_try.port_id='icahost'")
era_counts("channel_open_try:transfer","channel_open_try.port_id='transfer'")
era_counts("channel_close_confirm:icahost","channel_close_confirm.port_id='icahost'")
era_counts("write_ack:icahost","write_acknowledgement.packet_dst_port='icahost'")
era_counts("fungible_token_packet:success_false","fungible_token_packet.success='false'")
json.dump(res,open(rpc.RAW+"/ibc_event_counts.json","w"),indent=1)
bs={}
for res_ in ["proposal_passed","proposal_rejected","proposal_failed","proposal_dropped","expedited_proposal_rejected"]:
    for ev in ["active_proposal","inactive_proposal"]:
        q=f"\"{ev}.proposal_result='{res_}' AND block.height>=6680340 AND block.height<=15116658\""
        d=rpc.get("/block_search",{"query":q,"per_page":"50","page":"1","order_by":'"asc"'})
        if "result" in d:
            bs[f"{ev}:{res_}"]={"total":d["result"]["total_count"],"heights":[int(b["block"]["header"]["height"]) for b in d["result"]["blocks"]]}
        else: bs[f"{ev}:{res_}"]={"err":str(d)[:200]}
        print(ev,res_,bs[f"{ev}:{res_}"],flush=True)
json.dump(bs,open(rpc.RAW+"/gov_block_search.json","w"),indent=1)
print("reqs",rpc.NREQ[0])
