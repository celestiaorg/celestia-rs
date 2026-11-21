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
                _=connection(socket,bridge,auth.clone(), permissions.into())=>{}
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

#[derive(Default)]
struct Streams {
    subs: HashMap<String, Subscription>,
    next_channel: u64,
    calls: tokio::task::JoinSet<(String, String)>,
    requests: HashMap<String, tokio::task::AbortHandle>,
}

impl Streams {
    fn request(
        &mut self,
        bridge: &Arc<Bridge>,
        auth: &Arc<Auth>,
        perms: &Arc<[String]>,
        bytes: Vec<u8>,
    ) -> Result<Option<String>> {
        let request = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
        let method = request["method"].as_str().unwrap_or_default();
        if method == "xrpc.cancel" {
            let Some(id) = request["params"].get(0).map(Value::to_string) else {
                return Ok(None);
            };
            if let Some(call) = self.requests.remove(&id) {
                call.abort();
            }
            return Ok(self.subs.remove(&id).map(|sub| closed(sub.channel)));
        }
        let id = &request["id"];
        let key = id.to_string();
        if self.requests.contains_key(&key) || self.subs.contains_key(&key) {
            return Ok(Some(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32600,"message":"duplicate request ID"}}).to_string()));
        }
        if !matches!(
            method,
            "header.Subscribe" | "blob.Subscribe" | "blob.SubscribeFromStartHeight"
        ) {
            if self.calls.len() >= 16 {
                return Ok(Some(json!({"jsonrpc":"2.0","id":id,"error":{"code":1,"message":"too many requests"}}).to_string()));
            }
            let (bridge, auth, perms) = (bridge.clone(), auth.clone(), perms.clone());
            let request_id = key.clone();
            let handle = self
                .calls
                .spawn(async move { (request_id, dispatch(bridge, &auth, &perms, &bytes).await) });
            self.requests.insert(key, handle);
            return Ok(None);
        }
        let mut reply = if request["jsonrpc"] != "2.0" || !(id.is_string() || id.is_number()) {
            json!({"error":{"code":-32600,"message":"invalid request"}})
        } else if !perms.iter().any(|p| p == "read") {
            json!({"error":{"code":1,"message":"subscription requires read permission"}})
        } else if self.subs.len() >= 16 {
            json!({"error":{"code":1,"message":"too many subscriptions"}})
        } else {
            let empty = json!([]);
            let params = request
                .get("params")
                .filter(|v| !v.is_null())
                .unwrap_or(&empty);
            let channel = self.next_channel;
            match Subscription::new(bridge, method, params, channel) {
                Ok(sub) => {
                    self.next_channel = channel.checked_add(1).context("channel ID exhausted")?;
                    self.subs.insert(id.to_string(), sub);
                    json!({"result":channel})
                }
                Err(error) => json!({"error":{"code":1,"message":error.to_string()}}),
            }
        };
        reply["jsonrpc"] = json!("2.0");
        reply["id"] = id.clone();
        Ok(Some(reply.to_string()))
    }
}

async fn connection(
    mut socket: WebSocket,
    bridge: Arc<Bridge>,
    auth: Arc<Auth>,
    perms: Arc<[String]>,
) -> Result<()> {
    let mut streams = Streams::default();
    let mut tick = tokio::time::interval(Duration::from_millis(200));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            frame=socket.recv()=>{
                let Some(frame)=frame else {return Ok(())};
                let bytes=match frame? {Message::Text(t)=>t.into_bytes(),Message::Binary(b)=>b,Message::Close(_)=>return Ok(()),_=>continue};
                if let Some(reply) = streams.request(&bridge, &auth, &perms, bytes)? {
                    send(&mut socket, reply).await?;
                }
            }
            Some(result) = streams.calls.join_next_with_id(), if !streams.calls.is_empty() => {
                match result {
                    Ok((task, (key, reply))) if streams.requests.get(&key).is_some_and(|handle|handle.id()==task) => {
                        streams.requests.remove(&key);
                        send(&mut socket,reply).await?;
                    }
                    Err(error) if !error.is_cancelled() => return Err(error.into()),
                    _ => {}
                }
            }
            _=tick.tick(),if !streams.subs.is_empty()=>{
                let mut ended=Vec::new();
                for (id,sub) in &mut streams.subs {
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
                for id in ended {if let Some(sub)=streams.subs.remove(&id){send(&mut socket,closed(sub.channel)).await?;}}
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
