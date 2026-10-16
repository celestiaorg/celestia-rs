//! `countExecutableMsgs` (celestia-app v10 `app/msg_count.go`): the weight
//! of a tx's messages against the per-block SDK message limit.

use celestia_proto::Any;
use celestia_proto::cosmos::authz::v1beta1::MsgExec;
use celestia_proto::ibc::applications::interchain_accounts::host::v1::MsgModuleQuerySafe;
use celestia_proto::ibc::core::channel::v1::{Channel, MsgRecvPacket, Packet};
use prost::{Message, Name};
use sdk::Context;

/// `appconsts.MaxSDKMessages`.
pub const MAX_SDK_MESSAGES: usize = 600;

/// `icatypes.HostPortID`.
const ICA_HOST_PORT: &str = "icahost";

/// Counting needed state that a zeroed Go context does not have (Go
/// panics; see [`count_executable_msgs`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoStore;

/// `countExecutableMsgs`. `ctx` is `None` when Go's context is
/// `sdk.Context{}` (an ante decorator returned it): reading the channel
/// then panics, reported as [`NoStore`].
pub fn count_executable_msgs(
    mut ctx: Option<&mut Context<'_>>,
    msgs: &[Any],
) -> Result<usize, NoStore> {
    let mut count = 0;
    for msg in msgs {
        count += msg_weight(ctx.as_deref_mut(), msg)?;
        if count > MAX_SDK_MESSAGES {
            return Ok(count);
        }
    }
    Ok(count)
}

fn msg_weight(mut ctx: Option<&mut Context<'_>>, msg: &Any) -> Result<usize, NoStore> {
    if msg.type_url == MsgExec::type_url() {
        let Ok(exec) = MsgExec::decode(msg.value.as_slice()) else {
            return Ok(1);
        };
        // GetMessages succeeds for every tx that decoded (the decoder
        // unpacked each inner Any).
        let mut n = 1;
        for m in &exec.msgs {
            n += leaf_weight(ctx.as_deref_mut(), m)?;
        }
        return Ok(n);
    }
    leaf_weight(ctx, msg)
}

fn leaf_weight(ctx: Option<&mut Context<'_>>, msg: &Any) -> Result<usize, NoStore> {
    if msg.type_url == MsgModuleQuerySafe::type_url() {
        return Ok(
            MsgModuleQuerySafe::decode(msg.value.as_slice()).map_or(1, |m| 1 + m.requests.len())
        );
    }
    if msg.type_url == MsgRecvPacket::type_url() {
        let packet = MsgRecvPacket::decode(msg.value.as_slice())
            .ok()
            .and_then(|m| m.packet)
            .unwrap_or_default();
        return Ok(1 + count_ica_packet_msgs(ctx, &packet)?);
    }
    Ok(1)
}

/// `data.UnmarshalJSON` (`InterchainAccountPacketData`, jsonpb) with
/// `type == EXECUTE_TX`; returns its `data`.
fn ica_packet_data(bz: &[u8]) -> Option<Vec<u8>> {
    let (ty, data) = x_icahost::host::packet_data(bz)?;
    (ty == 1).then_some(data)
}

/// ibc-go `host.ChannelIdentifierValidator`.
fn valid_channel_id(id: &str) -> bool {
    !id.trim().is_empty()
        && !id.contains('/')
        && (8..=64).contains(&id.len())
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._+-#[]<>".contains(&c))
}

/// `channelKeeper.GetAppVersion`.
fn app_version(ctx: &mut Context<'_>, port: &str, channel: &str) -> Option<String> {
    let key = format!("channelEnds/ports/{port}/channels/{channel}");
    let bz = ctx.multistore().get("ibc", key.as_bytes()).ok()??;
    Channel::decode(bz.as_slice()).ok().map(|c| c.version)
}

/// `icatypes.MetadataFromVersion` → `Encoding`.
fn metadata_encoding(version: &str) -> Option<String> {
    x_icahost::host::metadata_from_version(version)
        .ok()
        .map(|m| m.encoding)
}

/// `countICAPacketMsgs`.
fn count_ica_packet_msgs(ctx: Option<&mut Context<'_>>, packet: &Packet) -> Result<usize, NoStore> {
    if packet.destination_port != ICA_HOST_PORT {
        return Ok(0);
    }
    let Some(data) = ica_packet_data(&packet.data) else {
        return Ok(0);
    };
    if !valid_channel_id(&packet.destination_channel) {
        return Ok(MAX_SDK_MESSAGES);
    }
    let ctx = ctx.ok_or(NoStore)?;
    let Some(version) = app_version(ctx, &packet.destination_port, &packet.destination_channel)
    else {
        return Ok(MAX_SDK_MESSAGES);
    };
    let Some(encoding) = metadata_encoding(&version) else {
        return Ok(MAX_SDK_MESSAGES);
    };
    let count = match encoding.as_str() {
        "proto3" => count_proto_msgs(&data),
        "proto3json" => count_json_msgs(&data),
        _ => None,
    };
    Ok(count.unwrap_or(MAX_SDK_MESSAGES))
}

fn uvarint(b: &[u8]) -> Option<(u64, usize)> {
    let mut v = 0u64;
    for (i, &byte) in b.iter().enumerate().take(10) {
        if i == 9 && byte > 1 {
            return None;
        }
        v |= u64::from(byte & 0x7f) << (7 * i);
        if byte < 0x80 {
            return Some((v, i + 1));
        }
    }
    None
}

/// `countProtoMsgs`: entries of field 1 (bytes) of a proto3 `CosmosTx`.
fn count_proto_msgs(mut bz: &[u8]) -> Option<usize> {
    let mut count = 0;
    while !bz.is_empty() {
        let (tag, n) = uvarint(bz)?;
        bz = &bz[n..];
        let num = tag >> 3;
        if num == 0 || num > (1 << 29) - 1 {
            return None;
        }
        let n = match tag & 7 {
            0 => uvarint(bz)?.1,
            1 => (bz.len() >= 8).then_some(8)?,
            2 => {
                let (len, n) = uvarint(bz)?;
                let end = n.checked_add(usize::try_from(len).ok()?)?;
                (end <= bz.len()).then_some(end)?
            }
            5 => (bz.len() >= 4).then_some(4)?,
            _ => return None,
        };
        if num == 1 && tag & 7 == 2 {
            count += 1;
        }
        bz = &bz[n..];
    }
    Some(count)
}

/// `countJSONMsgs`: `encoding/json` `Unmarshal` into
/// `struct{ Messages []struct{} }`. Go validates the whole input first
/// (`checkValid`: at most 10 000 nested levels, also inside values the
/// decoder skips), reads invalid UTF-8 inside a string as U+FFFD, matches
/// each object key to the field exactly or by `EqualFold` (`foldName`), lets
/// the last matching key win (`null` empties the slice), and fails the
/// whole call on any type mismatch even when a later key overwrites it.
#[must_use]
pub fn count_json_msgs(bz: &[u8]) -> Option<usize> {
    use gojson::Kind;
    let v = gojson::parse(bz).ok()?;
    let members = match &v.kind {
        Kind::Null => return Some(0),
        Kind::Object(m) => m,
        _ => return None,
    };
    let (mut count, mut ok) = (0, true);
    for (k, val) in members {
        if k != "messages" && !gostr::equal_fold(&gostr::from_go_bytes(k.as_bytes()), "messages") {
            continue;
        }
        match &val.kind {
            Kind::Null => count = 0,
            // Each element is a `struct{}`: an object (members skipped) or
            // `null`; anything else is a type error.
            Kind::Array(items) => {
                ok &= items
                    .iter()
                    .all(|e| matches!(e.kind, Kind::Object(_) | Kind::Null));
                count = items.len();
            }
            _ => ok = false,
        }
    }
    ok.then_some(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proto_and_json_counts() {
        assert_eq!(count_proto_msgs(&[0x0a, 0x00, 0x0a, 0x01, 0x05]), Some(2));
        assert_eq!(count_proto_msgs(&[0x0b]), None);
        assert_eq!(count_json_msgs(br#"{"messages":[{},{}]}"#), Some(2));
        assert_eq!(count_json_msgs(br#"{"Messages":[{}]}"#), Some(1));
        assert_eq!(count_json_msgs(b"[1]"), None);
        assert_eq!(count_json_msgs(b"null"), Some(0));
        assert_eq!(count_json_msgs(b"{\"messages\":[1]}"), None);
        assert_eq!(count_json_msgs(b"{\"messages\":[{},null]} x"), None);
        // encoding/json: invalid UTF-8 in a string is U+FFFD; keys match
        // by EqualFold (long s); the last matching key wins; a type error
        // fails the call even when a later key overwrites the field; a
        // lone surrogate escape is no error.
        assert_eq!(
            count_json_msgs(b"{\"messages\":[{\"memo\":\"m\xffo\"}]}"),
            Some(1)
        );
        assert_eq!(
            count_json_msgs("{\"me\u{17f}\u{17f}ages\":[{}]}".as_bytes()),
            Some(1)
        );
        assert_eq!(
            count_json_msgs(br#"{"messages":[{}],"MESSAGES":[{},{}]}"#),
            Some(2)
        );
        assert_eq!(
            count_json_msgs(br#"{"messages":[{}],"Messages":null}"#),
            Some(0)
        );
        assert_eq!(
            count_json_msgs(br#"{"messages":"x","messages":[{}]}"#),
            None
        );
        assert_eq!(
            count_json_msgs(br#"{"\ud800":1,"messages":[{}],"x":"\udc00"}"#),
            Some(1)
        );
        assert_eq!(count_json_msgs(br#"{"messages":[{"a":1e400}]}"#), Some(1));
        // `checkValid`'s 10 000-level limit, also inside skipped values
        // (Go v10.4.0 oracle, `json_count`).
        let deep = |d: usize| format!("{}0{}", "[".repeat(d), "]".repeat(d));
        let at = |d: usize| format!(r#"{{"messages":[{{}}],"x":{}}}"#, deep(d - 1));
        assert_eq!(count_json_msgs(at(129).as_bytes()), Some(1));
        assert_eq!(count_json_msgs(at(10_000).as_bytes()), Some(1));
        assert_eq!(count_json_msgs(at(10_001).as_bytes()), None);
        let inner = format!(r#"{{"messages":[{{"a":{}}}]}}"#, deep(9_998));
        assert_eq!(count_json_msgs(inner.as_bytes()), None);
        assert_eq!(
            ica_packet_data(b"{\"type\":\"TYPE_EXECUTE_TX\",\"data\":\"AQI=\",\"memo\":\"\xff\"}"),
            Some(vec![1, 2])
        );
        assert!(valid_channel_id("channel-0"));
        assert!(!valid_channel_id("chan"));
        assert_eq!(
            ica_packet_data(br#"{"type":"TYPE_EXECUTE_TX","data":"AQI="}"#),
            Some(vec![1, 2])
        );
    }
}
