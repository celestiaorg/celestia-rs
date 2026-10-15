//! Field-level description of two differing `SimulateResponse`s (for the
//! execution differential's reports).

use celestia_proto::cosmos::tx::v1beta1::SimulateResponse;
use prost::Message as _;

/// The first differing part of two `SimulateResponse` encodings.
#[must_use]
pub fn describe(go: &[u8], rust: &[u8]) -> String {
    let (Ok(g), Ok(r)) = (SimulateResponse::decode(go), SimulateResponse::decode(rust)) else {
        return "undecodable".into();
    };
    if g.gas_info != r.gas_info {
        return format!("gas_info: go={:?} rust={:?}", g.gas_info, r.gas_info);
    }
    let (g, r) = (g.result.unwrap_or_default(), r.result.unwrap_or_default());
    #[allow(deprecated)]
    if g.data != r.data {
        return format!(
            "data: go={} rust={}",
            hex::encode(&g.data),
            hex::encode(&r.data)
        );
    }
    if g.log != r.log {
        return format!("log: go={:?} rust={:?}", g.log, r.log);
    }
    if g.msg_responses != r.msg_responses {
        return format!(
            "msg_responses: go={:?} rust={:?}",
            g.msg_responses, r.msg_responses
        );
    }
    if g.events.len() != r.events.len() {
        return format!(
            "events: go {} rust {}: go={:?} rust={:?}",
            g.events.len(),
            r.events.len(),
            g.events.iter().map(|e| &e.r#type).collect::<Vec<_>>(),
            r.events.iter().map(|e| &e.r#type).collect::<Vec<_>>()
        );
    }
    for (i, (a, b)) in g.events.iter().zip(&r.events).enumerate() {
        if a != b {
            return format!("event {i}: go={a:?} rust={b:?}");
        }
    }
    "encoding only".into()
}
