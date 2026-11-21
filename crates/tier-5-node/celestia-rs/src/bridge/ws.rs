//! Channel transport used by go-jsonrpc. Each socket has its own cancellable cursors.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use super::{Bridge, Query, auth::Auth, credentials, dispatch, read_blobs};
use anyhow::{Context, Result, ensure};
use axum::{
    Extension,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use base64::Engine as _;
use rpc::json::J;
use serde_json::{Value, json};

pub(super) async fn upgrade(
    State(bridge): State<Arc<Bridge>>,
    Extension(auth): Extension<Arc<Auth>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let changed = auth.subscribe();
    let Ok((token, permissions)) = credentials(&auth, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let token = token.map(str::to_owned);
    ws.max_message_size(64 * 1024)
        .max_frame_size(64 * 1024)
        .on_upgrade(move |socket| async move {
            tokio::select! {
                _=connection(socket,bridge,&auth,&permissions)=>{}
                ()=auth.revoked(changed,token.as_deref())=>{}
            }
        })
}

struct Subscription {
    channel: u64,
    next: i64,
    ns: Option<shares::Namespace>,
}

impl Subscription {
    fn new(bridge: &Bridge, method: &str, params: &Value, channel: u64) -> Result<Self> {
        let head = bridge.blocks.height();
        let args = params.as_array().context("invalid params")?;
        let mut next = head.checked_add(1).context("height overflow")?;
        let ns = match (method, args.len()) {
            ("header.Subscribe", 0) => None,
            ("blob.Subscribe", 1) | ("blob.SubscribeFromStartHeight", 2) => {
                let raw = base64::engine::general_purpose::STANDARD
                    .decode(args[0].as_str().context("invalid namespace")?)?;
                if args.len() == 2 {
                    next = args[1].as_i64().context("invalid start height")?;
                    ensure!(
                        next > 0 && next >= bridge.blocks.base() && next <= head,
                        "start height unavailable"
                    );
                }
                Some(shares::Namespace::from_slice(&raw)?)
            }
            _ => anyhow::bail!("invalid subscription parameters"),
        };
        Ok(Self { channel, next, ns })
    }

    async fn event(&self, bridge: &Arc<Bridge>) -> Result<Option<String>> {
        let height = self.next;
        let ns = self.ns;
        if height > bridge.blocks.height() {
            return Ok(None);
        }
        let Ok(permit) = bridge.worker.clone().try_acquire_owned() else {
            return Ok(None);
        };
        let bridge = bridge.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let value = if let Some(ns) = ns {
                let block = bridge.committed(height)?;
                let blobs = read_blobs(&block.eds, &block.dah, ns)?
                    .into_iter()
                    .map(|(_, blob)| blob)
                    .collect::<Vec<_>>();
                J::obj([
                    (
                        "Blobs",
                        if blobs.is_empty() {
                            J::Null
                        } else {
                            J::Arr(blobs)
                        },
                    ),
                    ("Height", J::num(height)),
                    ("Header", raw_header(&block.header)?),
                ])
            } else {
                bridge.read(Query::Header(height))?
            };
            Ok(Some(value.encode()))
        })
        .await?
    }
}

async fn send(socket: &mut WebSocket, data: String) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(15), socket.send(Message::Text(data))).await??;
    Ok(())
}

async fn connection(
    mut socket: WebSocket,
    bridge: Arc<Bridge>,
    auth: &Auth,
    perms: &[String],
) -> Result<()> {
    let mut subs = HashMap::<String, Subscription>::new();
    let mut channel = 0u64;
    let mut tick = tokio::time::interval(Duration::from_millis(200));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            frame=socket.recv()=>{
                let Some(frame)=frame else {return Ok(())};
                let bytes=match frame? {Message::Text(t)=>t.into_bytes(),Message::Binary(b)=>b,Message::Close(_)=>return Ok(()),_=>continue};
                let request:Value=match serde_json::from_slice(&bytes){Ok(v)=>v,Err(_)=>{
                    send(&mut socket,dispatch(bridge.clone(),auth,perms,&bytes).await).await?;continue;
                }};
                let method=request["method"].as_str().unwrap_or("");
                if method=="xrpc.cancel" {
                    if let Some(id)=request["params"].get(0) {
                        if let Some(done)=subs.remove(&id.to_string()){send(&mut socket,closed(done.channel)).await?;}
                    }
                    continue;
                }
                if !matches!(method,"header.Subscribe"|"blob.Subscribe"|"blob.SubscribeFromStartHeight"){
                    let result=dispatch(bridge.clone(),auth,perms,&bytes).await;
                    send(&mut socket,result).await?;continue;
                }
                let id=&request["id"];
                let response=if request["jsonrpc"]!="2.0" || !(id.is_string()||id.is_number()) || subs.contains_key(&id.to_string()) {
                    json!({"error":{"code":-32600,"message":"invalid request"}})
                }else if !perms.iter().any(|p|p=="read"){
                    json!({"error":{"code":1,"message":"subscription requires read permission"}})
                }else if subs.len()>=16{
                    json!({"error":{"code":1,"message":"too many subscriptions"}})
                }else{
                    let empty=json!([]);
                    let params=request.get("params").filter(|v|!v.is_null()).unwrap_or(&empty);
                    match Subscription::new(&bridge,method,params,channel){
                        Ok(sub)=>{subs.insert(id.to_string(),sub);channel=channel.checked_add(1).context("channel ID exhausted")?;json!({"result":channel-1})}
                        Err(e)=>json!({"error":{"code":1,"message":e.to_string()}}),
                    }
                };
                let mut response=response;response["jsonrpc"]=json!("2.0");response["id"]=id.clone();
                send(&mut socket,response.to_string()).await?;
            }
            _=tick.tick(),if !subs.is_empty()=>{
                let mut ended=Vec::new();
                for (id,sub) in &mut subs {
                    if sub.next<bridge.blocks.base(){ended.push(id.clone());continue}
                    match sub.event(&bridge).await {
                        Ok(Some(value))=>{
                            send(&mut socket,format!(r#"{{"jsonrpc":"2.0","method":"xrpc.ch.val","params":[{},{}]}}"#,sub.channel,value)).await?;
                            sub.next=sub.next.checked_add(1).context("height overflow")?;
                        }
                        Ok(None)=>{}
                        Err(e)=>tracing::debug!(height=sub.next,error=%e,"subscription read deferred"),
                    }
                }
                for id in ended {if let Some(sub)=subs.remove(&id){send(&mut socket,closed(sub.channel)).await?;}}
            }
        }
    }
}

fn closed(id: u64) -> String {
    json!({"jsonrpc":"2.0","method":"xrpc.ch.close","params":[id]}).to_string()
}

// SubscriptionResponse.Header is marshaled with Go encoding/json. ExtendedHeader
// uses CometBFT JSON, which represents the same 64-bit fields as strings.
fn raw_header(h: &ctypes::Header) -> Result<J> {
    let mut value: Value = serde_json::from_str(&rpc::encode::header(h).encode())?;
    value["height"] = json!(h.height);
    value["version"]["block"] = json!(h.version.block);
    value["version"]["app"] = json!(h.version.app);
    Ok(J::Raw(value.to_string()))
}
