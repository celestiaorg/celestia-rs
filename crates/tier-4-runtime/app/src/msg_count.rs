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

/// `countExecutableMsgs` over `msgs` as `GetMsgs` returns them
/// ([`sdk::tx::Tx::gogo_msgs`]). `ctx` is `None` when Go's context is
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
        // unpacked each inner Any). The tx keeps inner messages as
        // received: count their gogoproto decodings, as Go does.
        let mut n = 1;
        for m in &exec.msgs {
            let m = txdecode::prost_compatible(m).unwrap_or_else(|| m.clone());
            n += leaf_weight(ctx.as_deref_mut(), &m)?;
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
        // protowire `ConsumeTag`: field numbers up to `math.MaxInt32`.
        if num == 0 || num > i32::MAX as u64 {
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

    /// A `MsgRecvPacket` of an ICA `EXECUTE_TX` packet with a protobuf key
    /// past `u32::MAX` (nightly fuzz-exec, deliver `ibc-edge` seed 3020):
    /// prost rejects the bytes as received, gogoproto decodes them, so Go
    /// reads the packet (and its channel) where a prost decode counted one
    /// message.
    const RECV_PACKET_WIDE_KEY_TX: &[&str] = &[
        "0a99080a96080a222f6962632e636f72652e6368616e6e656c2e76312e4d7367526563765061636b657412ef07f8ffff",
        "ff3f010aa1010808123d696361636f6e74726f6c6c65722d63656c6573746961317a376e677a646a3336303670787371",
        "76376a33646c70707963346533687967337279386d6c361a0a6368616e6e656c2d37372207696361686f73742a096368",
        "616e6e656c2d3232327b2264617461223a222f77453d222c226d656d6f223a22222c2274797065223a22545950455f45",
        "5845435554455f5458227d3a004080bca38fe6f290ed18128a060a88040a85040a6f636f6d6d69746d656e74732f706f",
        "7274732f696361636f6e74726f6c6c65722d63656c6573746961317a376e677a646a333630367078737176376a33646c",
        "70707963346533687967337279386d6c362f6368616e6e656c732f6368616e6e656c2d37372f73657175656e6365732f",
        "38122038e09095f2376dcbac78cfdd112f40db01bf8e7949cf6615b8c2b0ca4addb5fb1a0c0801180120012a040002a0",
        "02222a080112260204a0022054a67aa3a16a1991d0a5301aa94d0bb422fd3cb3d7adeb4e07c5d07a6cf16c2a20222a08",
        "0112260406a00220bb3f0b2ab016eb6fa39e62192c8abd7005701389a42a5fbc13dda615cbe14cae20222a0801122606",
        "0aa00220308490bde3d79e0e893bc9069f53914e9acdc8fdfc0cd13d08c03dfcd65cd84520222a080112260812a00220",
        "1e77bc660823b156f1aedf6f81aab220162f4ca11c2f5a9894e2367824f57d4d20222c080112050a2ca002201a2120f7",
        "60cf361192df1195ac6c740cf8a2000a3e38444053d80ad26a22211884cc77222a080112260e66a00220654e381bfeb3",
        "503b0054624236813d406d7c4739ab394a3a08cdf6b6d80fda2820222b0801122710e601a00220b7bcd94bcb85d0f74d",
        "5af3827de873efd6938fb41d05de07b992f29d84dafc7520222b0801122712f202a00220a3ae103512d85b18875a086c",
        "d9f8f06396a5820ab7f467868eb4fdbe2b236561200afc010af9010a03696263122050bbb9f26ece7af5bf2aa2ebbc1c",
        "4ebdb7760e5f1697904185665c00e864ca6b1a090801180120012a0100222708011201011a2003d82aa517051847a4bf",
        "891c7d326e4e0d22a3e4c3e7d14bc142a2731b58604f22250801122101c79a386cfbf13f5f09038bd1e798c16eec9b67",
        "8ba1d6a9807050519fc240c21622250801122101824b8fe40d0c9b2b3ebc3cee296aac01f7cbeb3bbd840e38c255e300",
        "663eee5c22250801122101031f969cb5c7a5afd3f041faef0ceb3fc7455df48a1d7dc2b7dfe2321ca8ff4d2227080112",
        "01011a201d39e1ea0f6c663ad7b33609b10bfb1fcd8704f6323396de95d198ce1958db9e1a050801109101222f63656c",
        "65737469613136726b35377567337a366a30726d30337a7a657775686d76346430753863376b33663775773012680a50",
        "0a460a1f2f636f736d6f732e63727970746f2e736563703235366b312e5075624b657912230a2103b94287437114b67c",
        "19738dae5c10de81dbdcd28c155c4037223c8049a4d0211b12040a020801180512140a0d0a0475746961120538303030",
        "31108092f4011a40ebafa50a0a8c6b2bb48c97be983abf318ba12615a42ab05886b7295ca63a12474f79174c2c0aa166",
        "6134c2e744a6945855a87761e11522644ffdc23a8803ad80",
    ];

    #[test]
    fn counts_the_gogoproto_decoding() {
        let raw = hex::decode(RECV_PACKET_WIDE_KEY_TX.concat()).unwrap();
        let tx = txdecode::decode(&raw).unwrap();
        assert_eq!(count_executable_msgs(None, tx.msgs()), Ok(1));
        assert_eq!(count_executable_msgs(None, tx.gogo_msgs()), Err(NoStore));
    }

    #[test]
    fn proto_and_json_counts() {
        assert_eq!(count_proto_msgs(&[0x0a, 0x00, 0x0a, 0x01, 0x05]), Some(2));
        assert_eq!(count_proto_msgs(&[0x0b]), None);
        // protowire `ConsumeTag`: field numbers up to 2^31 - 1 (2^29 is fine).
        assert_eq!(
            count_proto_msgs(&[0x0a, 0x00, 0x80, 0x80, 0x80, 0x80, 0x10, 0x00]),
            Some(1)
        );
        assert_eq!(
            count_proto_msgs(&[0x0a, 0x00, 0xf8, 0xff, 0xff, 0xff, 0x3f, 0x00]),
            Some(1)
        );
        assert_eq!(
            count_proto_msgs(&[0x0a, 0x00, 0x80, 0x80, 0x80, 0x80, 0x40, 0x00]),
            None
        );
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
