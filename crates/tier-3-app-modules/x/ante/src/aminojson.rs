//! `SIGN_MODE_LEGACY_AMINO_JSON` (x/tx v0.13.9 `signing/aminojson`): the
//! amino-JSON encoder driven by proto options, over `prost-reflect`.

use base64::Engine as _;
use prost_reflect::{
    DynamicMessage, ExtensionDescriptor, FieldDescriptor, MessageDescriptor, ReflectMessage, Value,
};
use sdk::tx::Tx;

use crate::signing::{SignerData, pool};

type Res<T> = Result<T, String>;

/// `SignModeHandler.GetSignBytes` for `SIGN_MODE_LEGACY_AMINO_JSON`.
pub fn sign_bytes(sd: &SignerData, tx: &Tx) -> Res<Vec<u8>> {
    // decode.RejectUnknownFields(body, allowUnknownNonCriticals=false) over
    // x/tx's descriptors (the interface registry's file resolver).
    if tx.body_has_unknown_non_criticals {
        return Err("unknown field found in TxBody".into());
    }
    reject_unknown_fields(
        &tx.body_bytes,
        &message("cosmos.tx.v1beta1.TxBody")?,
        10_000,
    )?;
    let body = &tx.body;
    if !body.extension_options.is_empty() || !body.non_critical_extension_options.is_empty() {
        return Err(
            "SIGN_MODE_LEGACY_AMINO_JSON does not support protobuf extension options: invalid request"
                .into(),
        );
    }
    if sd.address.is_empty() {
        return Err(
            "got empty address in SIGN_MODE_LEGACY_AMINO_JSON handler: invalid request".into(),
        );
    }
    let Some(fee) = &tx.auth_info.fee else {
        return Err("fee cannot be nil when tipper is not signer".into());
    };
    let enc = Encoder::new()?;
    let mut w = String::new();
    // AminoSignDoc, fields sorted by name.
    w.push_str("{\"account_number\":");
    w.push_str(&format!("\"{}\"", sd.account_number));
    w.push_str(",\"chain_id\":");
    json_string(&mut w, &sd.chain_id);
    // AminoSignFee: amount (legacy_coins, dont_omitempty), gas
    // (dont_omitempty), granter, payer (omitempty).
    w.push_str(",\"fee\":{\"amount\":");
    if fee.amount.is_empty() {
        w.push_str("[]");
    } else {
        let coin_desc = message("cosmos.base.v1beta1.Coin")?;
        w.push('[');
        for (i, c) in fee.amount.iter().enumerate() {
            if i > 0 {
                w.push(',');
            }
            // adapter.go: Amount = coin.Amount.String() (math.Int).
            let amount = if c.amount.is_empty() {
                "<nil>".to_owned()
            } else {
                math::Int::parse(&c.amount).map_or_else(|_| c.amount.clone(), |a| a.to_string())
            };
            let mut m = DynamicMessage::new(coin_desc.clone());
            m.set_field_by_name("denom", Value::String(c.denom.clone()));
            m.set_field_by_name("amount", Value::String(amount));
            enc.marshal_message(&m, &mut w)?;
        }
        w.push(']');
    }
    w.push_str(&format!(",\"gas\":\"{}\"", fee.gas_limit));
    if !fee.granter.is_empty() {
        w.push_str(",\"granter\":");
        json_string(&mut w, &fee.granter);
    }
    if !fee.payer.is_empty() {
        w.push_str(",\"payer\":");
        json_string(&mut w, &fee.payer);
    }
    w.push_str("},\"memo\":");
    json_string(&mut w, &body.memo);
    w.push_str(",\"msgs\":");
    if body.messages.is_empty() {
        w.push_str("null");
    } else {
        w.push('[');
        for (i, m) in body.messages.iter().enumerate() {
            if i > 0 {
                w.push(',');
            }
            enc.marshal_any(&m.type_url, &m.value, &mut w)?;
        }
        w.push(']');
    }
    w.push_str(&format!(",\"sequence\":\"{}\"", sd.sequence));
    if body.timeout_height != 0 {
        w.push_str(&format!(",\"timeout_height\":\"{}\"", body.timeout_height));
    }
    w.push('}');
    Ok(gostr::to_go_bytes(&w).into_owned())
}

struct Exts {
    name: ExtensionDescriptor,
    message_encoding: ExtensionDescriptor,
    encoding: ExtensionDescriptor,
    dont_omitempty: ExtensionDescriptor,
    field_name: ExtensionDescriptor,
    oneof_name: ExtensionDescriptor,
    scalar: ExtensionDescriptor,
    customtype: ExtensionDescriptor,
}

/// The amino-JSON `Encoder` (default options: sorted fields, enums as
/// numbers, no map support).
pub struct Encoder {
    ext: Exts,
}

fn ext(name: &str) -> Res<ExtensionDescriptor> {
    pool()
        .get_extension_by_name(name)
        .ok_or_else(|| format!("extension {name} not found"))
}

fn message(name: &str) -> Res<MessageDescriptor> {
    pool()
        .get_message_by_name(name)
        .ok_or_else(|| format!("can't resolve type URL /{name}"))
}

/// Fields whose message type the interface registry's file resolver
/// resolves to a placeholder descriptor (ibc-go v8.7.2's `tx.proto` is
/// built without `upgrade.proto`): x/tx sees no fields in them, so any
/// content is an unknown field (`fuzzoracle placeholders`, the fields
/// reachable from txs; identical at both app versions).
const PLACEHOLDER_FIELDS: &[(&str, &str)] = &[
    (
        "ibc.core.channel.v1.MsgChannelUpgradeAck",
        "counterparty_upgrade",
    ),
    (
        "ibc.core.channel.v1.MsgChannelUpgradeCancel",
        "error_receipt",
    ),
    (
        "ibc.core.channel.v1.MsgChannelUpgradeConfirm",
        "counterparty_upgrade",
    ),
    ("ibc.core.channel.v1.MsgChannelUpgradeInit", "fields"),
    (
        "ibc.core.channel.v1.MsgChannelUpgradeTry",
        "counterparty_upgrade_fields",
    ),
];

fn wire_type_name(wt: u8) -> String {
    match wt {
        0 => "varint".into(),
        1 => "fixed64".into(),
        2 => "bytes".into(),
        3 => "start_group".into(),
        4 => "end_group".into(),
        5 => "fixed32".into(),
        t => format!("unknown type: {t}"),
    }
}

/// Whether every `type_url` occurrence in the `Any` bytes `b` is UTF-8.
fn any_type_urls_utf8(mut b: &[u8]) -> bool {
    while let Some((tag, m)) = read_varint(b) {
        b = &b[m..];
        let n = match tag & 7 {
            0 => read_varint(b).map(|(_, n)| (n, None)),
            1 => (b.len() >= 8).then_some((8, None)),
            5 => (b.len() >= 4).then_some((4, None)),
            2 => read_varint(b).and_then(|(l, n)| {
                let end = n.checked_add(usize::try_from(l).ok()?)?;
                (end <= b.len()).then_some((end, Some(n)))
            }),
            _ => None,
        };
        let Some((end, start)) = n else { return true };
        if tag >> 3 == 1
            && let Some(s) = start
            && std::str::from_utf8(&b[s..end]).is_err()
        {
            return false;
        }
        b = &b[end..];
    }
    true
}

fn read_varint(b: &[u8]) -> Option<(u64, usize)> {
    let mut v = 0u64;
    for (i, &x) in b.iter().enumerate().take(10) {
        v |= u64::from(x & 0x7f) << (7 * i);
        if x < 0x80 {
            return Some((v, i + 1));
        }
    }
    None
}

/// x/tx `decode.RejectUnknownFields(bz, desc, false, resolver)`: every
/// unknown field, critical or not, at any depth (through `Any`s) is an
/// error; `placeholder` descriptors have no fields.
fn reject_unknown_fields(bz: &[u8], desc: &MessageDescriptor, limit: usize) -> Res<()> {
    reject_unknown_in(bz, desc, false, limit)
}

fn reject_unknown_in(
    mut bz: &[u8],
    desc: &MessageDescriptor,
    placeholder: bool,
    limit: usize,
) -> Res<()> {
    if bz.is_empty() {
        return Ok(());
    }
    if limit == 0 {
        return Err("recursion limit reached".into());
    }
    while !bz.is_empty() {
        let Some((tag, m)) = read_varint(bz) else {
            return Err("invalid length".into());
        };
        let (num, wt) = (tag >> 3, (tag & 7) as u8);
        let field = if placeholder {
            None
        } else {
            u32::try_from(num).ok().and_then(|n| desc.get_field(n))
        };
        if field.is_none() {
            return Err(format!(
                "{}: {{TagNum: {num}, WireType:\"{}\"}}: unknown protobuf field",
                desc.full_name(),
                wire_type_name(wt)
            ));
        }
        bz = &bz[m..];
        let n = match wt {
            0 => read_varint(bz).map(|(_, n)| n),
            1 => (bz.len() >= 8).then_some(8),
            5 => (bz.len() >= 4).then_some(4),
            2 => read_varint(bz).and_then(|(l, n)| {
                let end = n.checked_add(usize::try_from(l).ok()?)?;
                (end <= bz.len()).then_some(end)
            }),
            _ => None,
        };
        let Some(n) = n else {
            return Err(format!(
                "could not consume field value for tagNum: {num}, wireType: \"{}\"",
                wire_type_name(wt)
            ));
        };
        let value = &bz[..n];
        bz = &bz[n..];
        let Some(field) = field else { continue };
        let prost_reflect::Kind::Message(sub) = field.kind() else {
            continue;
        };
        let Some((_, o)) = read_varint(value) else {
            return Err("could not consume length prefix".into());
        };
        let inner = &value[o..];
        if sub.full_name() == "google.protobuf.Any" {
            reject_unknown_in(inner, &sub, false, limit - 1)?;
            let any = <celestia_proto::Any as prost::Message>::decode(inner)
                .map_err(|e| e.to_string())?;
            // protobuf-go's `anypb.Any` (proto3) checks every occurrence
            // of the type URL for UTF-8 (the last one is kept).
            if !any_type_urls_utf8(inner) {
                return Err("string field contains invalid UTF-8".into());
            }
            let name = any.type_url.trim_start_matches('/');
            let d = pool()
                .get_message_by_name(name)
                .ok_or_else(|| format!("proto: not found: {name}"))?;
            reject_unknown_in(&any.value, &d, false, limit - 1)?;
            continue;
        }
        let ph = PLACEHOLDER_FIELDS
            .iter()
            .any(|(m, f)| *m == desc.full_name() && *f == field.name());
        reject_unknown_in(inner, &sub, ph, limit - 1)?;
    }
    Ok(())
}

/// The `amino.name` of hyperlane-cosmos v1.1.0's protos (celestia-app
/// v9.0.8) where it differs from v1.3.0's (vendored), at app version 9.
fn v9_amino_name(full_name: &str) -> Option<&'static str> {
    if !sdk::version::is_v9(sdk::version::app_version()) {
        return None;
    }
    match full_name {
        "hyperlane.core.interchain_security.v1.MsgSetRoutingIsmDomain" => {
            Some("hyperlane/v1/MsgCreateRoMsgSetRoutingIsmDomainutingIsm")
        }
        "hyperlane.core.post_dispatch.v1.MsgCreateIgp" => {
            Some("hyperlane/v1/MsgCreateInterchainGasPaymaster")
        }
        "hyperlane.core.post_dispatch.v1.MsgCreateNoopHook" => {
            Some("hyperlane/v1/MsgCreateMerkleTreeHook")
        }
        _ => None,
    }
}

fn str_ext(opts: &DynamicMessage, e: &ExtensionDescriptor) -> Option<String> {
    if !opts.has_extension(e) {
        return None;
    }
    opts.get_extension(e).as_str().map(ToOwned::to_owned)
}

impl Encoder {
    /// Creates the encoder.
    pub fn new() -> Res<Self> {
        Ok(Self {
            ext: Exts {
                name: ext("amino.name")?,
                message_encoding: ext("amino.message_encoding")?,
                encoding: ext("amino.encoding")?,
                dont_omitempty: ext("amino.dont_omitempty")?,
                field_name: ext("amino.field_name")?,
                oneof_name: ext("amino.oneof_name")?,
                scalar: ext("cosmos_proto.scalar")?,
                customtype: ext("gogoproto.customtype")?,
            },
        })
    }

    /// `marshalAny`: resolves the packed type and writes
    /// `{"type":<amino name or type URL>,"value":…}`.
    pub fn marshal_any(&self, type_url: &str, value: &[u8], w: &mut String) -> Res<()> {
        // `proto.Unmarshal` into the resolved type: the generated (pulsar)
        // unmarshal for the types protoregistry knows, `dynamicpb` (UTF-8
        // checked in proto3 strings, strict varints) for the rest, as in
        // `GetSigners` (finding 1).
        celestia_proto::signers::unpack_check(type_url, value).map_err(|e| {
            e.trim_start_matches(&format!("cannot unmarshal msg {type_url}: "))
                .to_owned()
        })?;
        let name = type_url.rsplit('/').next().unwrap_or(type_url);
        let desc = message(name)?;
        let value = go_form_strings(&desc, value, 0).unwrap_or_else(|| value.to_vec());
        let value = if celestia_proto::getsigners_schema::PROTOV2_TYPES
            .binary_search(&type_url)
            .is_ok()
        {
            pulsar_last_wins(&desc, &value, 0).unwrap_or(value)
        } else {
            value
        };
        let msg = DynamicMessage::decode(desc, value.as_slice()).map_err(|e| e.to_string())?;
        self.begin_marshal(&msg, w, true)
    }

    fn begin_marshal(&self, msg: &DynamicMessage, w: &mut String, is_any: bool) -> Res<()> {
        let desc = msg.descriptor();
        let amino_name = v9_amino_name(desc.full_name())
            .map(ToOwned::to_owned)
            .or_else(|| str_ext(&desc.options(), &self.ext.name));
        let name = if is_any {
            Some(amino_name.unwrap_or_else(|| format!("/{}", desc.full_name())))
        } else {
            amino_name
        };
        if let Some(n) = &name {
            w.push_str(&format!("{{\"type\":\"{n}\",\"value\":"));
        }
        self.marshal_message(msg, w)?;
        if name.is_some() {
            w.push('}');
        }
        Ok(())
    }

    fn marshal_value(&self, v: &Value, w: &mut String) -> Res<()> {
        match v {
            Value::Message(m) => self.marshal_message(m, w),
            Value::Map(_) => Err("maps are not supported".into()),
            Value::List(l) => {
                if l.is_empty() {
                    w.push_str("null");
                    Ok(())
                } else {
                    self.marshal_list(l, w)
                }
            }
            Value::String(s) => {
                json_string(w, s);
                Ok(())
            }
            Value::Bool(b) => {
                w.push_str(if *b { "true" } else { "false" });
                Ok(())
            }
            Value::I32(i) => {
                w.push_str(&i.to_string());
                Ok(())
            }
            Value::U32(i) => {
                w.push_str(&i.to_string());
                Ok(())
            }
            Value::EnumNumber(i) => {
                w.push_str(&i.to_string());
                Ok(())
            }
            Value::Bytes(b) => {
                w.push('"');
                w.push_str(&base64::engine::general_purpose::STANDARD.encode(b));
                w.push('"');
                Ok(())
            }
            Value::I64(i) => {
                w.push_str(&format!("\"{i}\""));
                Ok(())
            }
            Value::U64(i) => {
                w.push_str(&format!("\"{i}\""));
                Ok(())
            }
            Value::F32(_) => Err("unknown type float32".into()),
            Value::F64(_) => Err("unknown type float64".into()),
        }
    }

    fn marshal_list(&self, l: &[Value], w: &mut String) -> Res<()> {
        w.push('[');
        for (i, v) in l.iter().enumerate() {
            if i > 0 {
                w.push(',');
            }
            self.marshal_value(v, w)?;
        }
        w.push(']');
        Ok(())
    }

    fn marshal_message(&self, msg: &DynamicMessage, w: &mut String) -> Res<()> {
        let desc = msg.descriptor();
        match desc.full_name() {
            "google.protobuf.Timestamp" => return marshal_timestamp(msg, w),
            "google.protobuf.Duration" => return marshal_duration(msg, w),
            "google.protobuf.Any" => {
                let type_url = msg
                    .get_field_by_name("type_url")
                    .and_then(|v| v.as_str().map(ToOwned::to_owned))
                    .unwrap_or_default();
                let value = msg
                    .get_field_by_name("value")
                    .and_then(|v| v.as_bytes().map(|b| b.to_vec()))
                    .unwrap_or_default();
                if type_url.is_empty() {
                    return Err("runtime error: slice bounds out of range [1:0]".into());
                }
                return self.marshal_any(&type_url, &value, w);
            }
            _ => {}
        }
        if let Some(enc) = str_ext(&desc.options(), &self.ext.message_encoding) {
            match enc.as_str() {
                "key_field" => return key_field_encoder(msg, w),
                "module_account" => return module_account_encoder(msg, w),
                "threshold_string" => return self.threshold_string_encoder(msg, w),
                _ => {}
            }
        }

        struct Entry {
            fd: FieldDescriptor,
            name: String,
            oneof: Option<(String, String, String)>, // (oneof full name, field name, type name)
        }
        let mut entries = Vec::new();
        for fd in desc.fields() {
            let name = str_ext(&fd.options(), &self.ext.field_name)
                .unwrap_or_else(|| fd.name().to_owned());
            let oneof = match fd.containing_oneof() {
                Some(o) => {
                    let type_name = str_ext(&fd.options(), &self.ext.oneof_name).ok_or_else(|| {
                        format!(
                            "field {} within a oneof must have the amino.oneof_type_name option set",
                            fd.name()
                        )
                    })?;
                    Some((
                        o.full_name().to_owned(),
                        to_camel(go_oneof_name(&o)),
                        type_name,
                    ))
                }
                None => None,
            };
            entries.push(Entry { fd, name, oneof });
        }
        entries.sort_by(|a, b| {
            let an = a.oneof.as_ref().map_or(&a.name, |o| &o.1);
            let bn = b.oneof.as_ref().map_or(&b.name, |o| &o.1);
            an.cmp(bn)
        });

        w.push('{');
        let mut first = true;
        let mut empty_oneof_written: Vec<String> = Vec::new();
        for e in &entries {
            let fd = &e.fd;
            let mut name = e.name.clone();
            let mut write_nil = false;
            if !msg.has_field(fd) {
                let oneof_unset = e.oneof.as_ref().is_some_and(|(full, field_name, _)| {
                    let set = fd
                        .containing_oneof()
                        .is_some_and(|o| o.fields().any(|f| msg.has_field(&f)));
                    let _ = full;
                    !set && !empty_oneof_written.contains(field_name)
                });
                if oneof_unset {
                    if let Some((_, field_name, _)) = &e.oneof {
                        name.clone_from(field_name);
                        empty_oneof_written.push(field_name.clone());
                    }
                    write_nil = true;
                } else if self.omit_empty(fd) {
                    continue;
                } else if fd.kind().as_message().is_some() && !fd.is_list() && !fd.is_map() {
                    return Err(format!(
                        "not supported: dont_omit_empty=true on invalid (nil?) message field: {name}"
                    ));
                }
            }
            if !first {
                w.push(',');
            }
            let is_oneof = e.oneof.is_some();
            if is_oneof
                && !write_nil
                && let Some((_, field_name, type_name)) = &e.oneof
            {
                w.push_str(&format!(
                    "\"{field_name}\":{{\"type\":\"{type_name}\",\"value\":{{"
                ));
            }
            json_string(w, &name);
            w.push(':');
            let value = msg.get_field(fd);
            if self.field_encoder(fd, &value, w)?.is_some() {
            } else if write_nil
                || (fd.is_list() && value.as_list().is_some_and(<[Value]>::is_empty))
            {
                w.push_str("null");
            } else {
                self.marshal_value(&value, w)?;
            }
            if is_oneof && !write_nil {
                w.push_str("}}");
            }
            first = false;
        }
        w.push('}');
        Ok(())
    }

    fn omit_empty(&self, fd: &FieldDescriptor) -> bool {
        let opts = fd.options();
        if opts.has_extension(&self.ext.dont_omitempty) {
            return !opts
                .get_extension(&self.ext.dont_omitempty)
                .as_bool()
                .unwrap_or(false);
        }
        true
    }

    /// `getFieldEncoder`; returns `Some(())` when a custom encoder wrote the
    /// value.
    fn field_encoder(&self, fd: &FieldDescriptor, v: &Value, w: &mut String) -> Res<Option<()>> {
        let opts = fd.options();
        if let Some(enc) = str_ext(&opts, &self.ext.encoding) {
            match enc.as_str() {
                "legacy_coins" => {
                    let Value::List(l) = v else {
                        return Err(format!("unsupported type {v:?}"));
                    };
                    if l.is_empty() {
                        w.push_str("[]");
                    } else {
                        self.marshal_list(l, w)?;
                    }
                    return Ok(Some(()));
                }
                "inline_json" => return Err("inline_json encoding is not supported".into()),
                _ => {}
            }
        }
        if let Some(scalar) = str_ext(&opts, &self.ext.scalar) {
            if scalar == "cosmos.Dec" {
                let ct = str_ext(&opts, &self.ext.customtype).unwrap_or_default();
                if ct != "cosmossdk.io/math.LegacyDec" {
                    return Ok(None);
                }
                let bytes: Vec<u8> = match v {
                    Value::String(s) => gostr::to_go_bytes(s).into_owned(),
                    Value::Bytes(b) => b.to_vec(),
                    other => return Err(format!("unsupported type {other:?}")),
                };
                if bytes.is_empty() {
                    json_string(w, "0");
                } else {
                    let d = math::LegacyDec::unmarshal(&bytes).map_err(|e| e.to_string())?;
                    json_string(w, &d.to_string());
                }
                return Ok(Some(()));
            }
            if scalar == "cosmos.Int" {
                match v {
                    Value::String(s) => {
                        json_string(w, if s.is_empty() { "0" } else { s });
                    }
                    Value::Bytes(b) => {
                        if b.is_empty() {
                            json_string(w, "0");
                        } else {
                            let i = math::Int::unmarshal(b).map_err(|e| e.to_string())?;
                            json_string(w, &i.to_string());
                        }
                    }
                    other => return Err(format!("unsupported type {other:?}")),
                }
                return Ok(Some(()));
            }
        }
        Ok(None)
    }

    fn threshold_string_encoder(&self, msg: &DynamicMessage, w: &mut String) -> Res<()> {
        w.push_str("{\"pubkeys\":");
        let pks = msg
            .get_field_by_name("public_keys")
            .map(std::borrow::Cow::into_owned);
        match pks.as_ref().and_then(Value::as_list) {
            Some(l) if !l.is_empty() => self.marshal_list(l, w)?,
            _ => w.push_str("[]"),
        }
        let t = msg
            .get_field_by_name("threshold")
            .and_then(|v| v.as_u32())
            .unwrap_or(0);
        w.push_str(&format!(",\"threshold\":\"{t}\"}}"));
        Ok(())
    }
}

fn key_field_encoder(msg: &DynamicMessage, w: &mut String) -> Res<()> {
    let key = msg
        .get_field_by_name("key")
        .ok_or("message encoder for key_field: no field named \"key\" found")?;
    let bz = key.as_bytes().map(|b| b.to_vec()).unwrap_or_default();
    if bz.is_empty() {
        w.push_str("null");
    } else {
        w.push('"');
        w.push_str(&base64::engine::general_purpose::STANDARD.encode(bz));
        w.push('"');
    }
    Ok(())
}

fn module_account_encoder(msg: &DynamicMessage, w: &mut String) -> Res<()> {
    if msg.descriptor().full_name() != "cosmos.auth.v1beta1.ModuleAccount" {
        return Err("moduleAccountEncoder: msg not a auth.ModuleAccount".into());
    }
    let name = msg
        .get_field_by_name("name")
        .and_then(|v| v.as_str().map(ToOwned::to_owned))
        .unwrap_or_default();
    let perms: Vec<String> = msg
        .get_field_by_name("permissions")
        .and_then(|v| {
            v.as_list().map(|l| {
                l.iter()
                    .filter_map(|x| x.as_str().map(ToOwned::to_owned))
                    .collect()
            })
        })
        .unwrap_or_default();
    let (mut address, mut number, mut sequence) = (String::new(), 0u64, 0u64);
    if let Some(fd) = msg.descriptor().get_field_by_name("base_account")
        && msg.has_field(&fd)
        && let Value::Message(ba) = msg.get_field(&fd).into_owned()
    {
        address = ba
            .get_field_by_name("address")
            .and_then(|v| v.as_str().map(ToOwned::to_owned))
            .unwrap_or_default();
        number = ba
            .get_field_by_name("account_number")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        sequence = ba
            .get_field_by_name("sequence")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
    }
    w.push_str(&format!("{{\"account_number\":{number},\"address\":"));
    json_string(w, &address);
    w.push_str(",\"name\":");
    json_string(w, &name);
    w.push_str(",\"permissions\":");
    if perms.is_empty() {
        w.push_str("null");
    } else {
        w.push('[');
        for (i, p) in perms.iter().enumerate() {
            if i > 0 {
                w.push(',');
            }
            json_string(w, p);
        }
        w.push(']');
    }
    w.push_str(&format!(",\"public_key\":\"\",\"sequence\":{sequence}}}"));
    Ok(())
}

fn int_field(msg: &DynamicMessage, name: &str) -> Res<i64> {
    let v = msg
        .get_field_by_name(name)
        .ok_or_else(|| format!("expected {name} field"))?;
    Ok(v.as_i64()
        .or_else(|| v.as_i32().map(i64::from))
        .unwrap_or(0))
}

fn marshal_timestamp(msg: &DynamicMessage, w: &mut String) -> Res<()> {
    let seconds = int_field(msg, "seconds")?;
    let nanos = int_field(msg, "nanos")?;
    if nanos < 0 {
        return Err("nanos must be non-negative on timestamp".into());
    }
    w.push('"');
    w.push_str(&format_rfc3339(seconds, nanos));
    w.push('"');
    Ok(())
}

/// `MaxDurationSeconds`.
const MAX_DURATION_SECONDS: i64 = i64::MAX / 1_000_000_000 - 1;

fn marshal_duration(msg: &DynamicMessage, w: &mut String) -> Res<()> {
    let seconds = int_field(msg, "seconds")?;
    if seconds > MAX_DURATION_SECONDS {
        return Err(format!(
            "{seconds} seconds would overflow an int64 when represented as nanoseconds"
        ));
    }
    let nanos = int_field(msg, "nanos")?;
    let total = nanos.wrapping_add(seconds.wrapping_mul(1_000_000_000));
    w.push_str(&format!("\"{total}\""));
    Ok(())
}

/// Go `time.Unix(s, n).UTC().Format(RFC3339 / RFC3339Nano)`.
#[must_use]
pub fn format_rfc3339(seconds: i64, nanos: i64) -> String {
    let secs = seconds + nanos.div_euclid(1_000_000_000);
    let nanos = nanos.rem_euclid(1_000_000_000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let year = if y < 0 {
        format!("-{:04}", -y)
    } else {
        format!("{y:04}")
    };
    let mut s = format!("{year}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}");
    if nanos != 0 {
        let frac = format!("{nanos:09}");
        s.push('.');
        s.push_str(frac.trim_end_matches('0'));
    }
    s.push('Z');
    s
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The upstream oneof name. The vendored protos rename oneofs that collide
/// with nested message names under prost; amino JSON encodes the Go name.
fn go_oneof_name(o: &prost_reflect::OneofDescriptor) -> &str {
    match o.full_name() {
        "cosmos.staking.v1beta1.StakeAuthorization.policy" => "validators",
        _ => o.name(),
    }
}

/// `strcase.ToCamel`.
fn to_camel(s: &str) -> String {
    let mut out = String::new();
    let mut cap_next = true;
    for (i, c) in s.chars().enumerate() {
        let _ = i;
        if c == '_' || c == ' ' || c == '-' || c == '.' {
            cap_next = true;
            continue;
        }
        if c.is_ascii_digit() {
            out.push(c);
            cap_next = true;
            continue;
        }
        if cap_next {
            out.push(c.to_ascii_uppercase());
        } else {
            out.push(c);
        }
        cap_next = false;
    }
    out
}

fn wire_varint(b: &mut &[u8]) -> Option<u64> {
    let mut v = 0u64;
    for i in 0..10 {
        let (&x, rest) = b.split_first()?;
        *b = rest;
        v |= u64::from(x & 0x7f) << (7 * i);
        if x < 0x80 {
            return Some(v);
        }
    }
    None
}

#[allow(clippy::cast_possible_truncation)]
fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// The message bytes with every string field (in nested messages too)
/// re-encoded in Go form (`gostr`), so `prost-reflect` decodes the strings
/// Go's protobuf-go unmarshal keeps (pulsar types do not check UTF-8; the
/// `dynamicpb` types were already checked by `GetSigners`). `None` when
/// the bytes are not well-formed (decoding then reports the error).
fn go_form_strings(desc: &MessageDescriptor, mut b: &[u8], depth: usize) -> Option<Vec<u8>> {
    use prost_reflect::Kind;
    if depth > 100 {
        return None;
    }
    let mut out = Vec::with_capacity(b.len());
    while !b.is_empty() {
        let start = b;
        let tag = wire_varint(&mut b)?;
        let num = u32::try_from(tag >> 3).ok()?;
        match tag & 7 {
            0 => {
                wire_varint(&mut b)?;
            }
            1 => b = b.get(8..)?,
            5 => b = b.get(4..)?,
            2 => {
                let len = usize::try_from(wire_varint(&mut b)?).ok()?;
                let payload = b.get(..len)?;
                b = &b[len..];
                let f = desc.get_field(num);
                let new: Option<Vec<u8>> = match f.as_ref().map(FieldDescriptor::kind) {
                    Some(Kind::String) => Some(gostr::from_go_bytes(payload).as_bytes().to_vec()),
                    Some(Kind::Message(m)) => Some(go_form_strings(&m, payload, depth + 1)?),
                    _ => None,
                };
                if let Some(n) = new {
                    put_varint(&mut out, tag);
                    put_varint(&mut out, n.len() as u64);
                    out.extend_from_slice(&n);
                    continue;
                }
            }
            // Groups and invalid wire types: left to the decoder.
            _ => return None,
        }
        out.extend_from_slice(&start[..start.len() - b.len()]);
    }
    Some(out)
}

/// The generated (pulsar) unmarshal of the types `protoregistry.GlobalTypes`
/// knows unmarshals each occurrence of a singular message field into a
/// fresh message: the last occurrence wins (`dynamicpb` and gogoproto
/// merge them). Rewrites `b` (a message of `desc`) to its last occurrences,
/// recursively (`Any` values are resolved again by `marshalAny`).
fn pulsar_last_wins(desc: &MessageDescriptor, b: &[u8], depth: usize) -> Option<Vec<u8>> {
    use prost_reflect::Kind;
    if depth > 100 {
        return None;
    }
    // (field number, start, end, payload range) of every field.
    type WireField = (u32, usize, usize, Option<(usize, usize)>);
    let mut fields: Vec<WireField> = Vec::new();
    let mut rest = b;
    while !rest.is_empty() {
        let start = b.len() - rest.len();
        let tag = wire_varint(&mut rest)?;
        let num = u32::try_from(tag >> 3).ok()?;
        let payload = match tag & 7 {
            0 => {
                wire_varint(&mut rest)?;
                None
            }
            1 => {
                rest = rest.get(8..)?;
                None
            }
            5 => {
                rest = rest.get(4..)?;
                None
            }
            2 => {
                let len = usize::try_from(wire_varint(&mut rest)?).ok()?;
                let p0 = b.len() - rest.len();
                rest = rest.get(len..)?;
                Some((p0, p0 + len))
            }
            _ => return None,
        };
        fields.push((num, start, b.len() - rest.len(), payload));
    }
    let mut out = Vec::with_capacity(b.len());
    for (i, &(num, start, end, payload)) in fields.iter().enumerate() {
        let fd = desc.get_field(num);
        let msg = match fd.as_ref().map(FieldDescriptor::kind) {
            Some(Kind::Message(m)) => Some(m),
            _ => None,
        };
        let (Some(fd), Some(m), Some((p0, p1))) = (fd, msg, payload) else {
            out.extend_from_slice(&b[start..end]);
            continue;
        };
        let singular = !fd.is_list() && !fd.is_map();
        if singular && fields[i + 1..].iter().any(|f| f.0 == num) {
            continue;
        }
        if m.full_name() == "google.protobuf.Any" {
            out.extend_from_slice(&b[start..end]);
            continue;
        }
        let inner = pulsar_last_wins(&m, &b[p0..p1], depth + 1)?;
        out.extend_from_slice(&b[start..p0 - varint_len((p1 - p0) as u64)]);
        put_varint(&mut out, inner.len() as u64);
        out.extend_from_slice(&inner);
    }
    Some(out)
}

fn varint_len(mut v: u64) -> usize {
    let mut n = 1;
    while v >= 0x80 {
        v >>= 7;
        n += 1;
    }
    n
}

/// Go `encoding/json` string encoding (HTML-safe escaping, `\b`/`\f`
/// short escapes as of Go 1.22) of the Go string `s` (Go form): each
/// invalid UTF-8 byte is written as `�`. `w` is a Go form.
pub fn json_string(w: &mut String, s: &str) {
    w.push('"');
    for r in gostr::runes(s) {
        let Some(c) = r else {
            w.push_str("\\ufffd");
            continue;
        };
        match c {
            '"' => w.push_str("\\\""),
            '\\' => w.push_str("\\\\"),
            '\n' => w.push_str("\\n"),
            '\r' => w.push_str("\\r"),
            '\t' => w.push_str("\\t"),
            '\u{8}' => w.push_str("\\b"),
            '\u{c}' => w.push_str("\\f"),
            '<' => w.push_str("\\u003c"),
            '>' => w.push_str("\\u003e"),
            '&' => w.push_str("\\u0026"),
            '\u{2028}' => w.push_str("\\u2028"),
            '\u{2029}' => w.push_str("\\u2029"),
            c if (c as u32) < 0x20 => w.push_str(&format!("\\u{:04x}", c as u32)),
            c => gostr::push_char(w, c),
        }
    }
    w.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339() {
        assert_eq!(format_rfc3339(0, 0), "1970-01-01T00:00:00Z");
        assert_eq!(
            format_rfc3339(1_700_000_000, 500_000_000),
            "2023-11-14T22:13:20.5Z"
        );
        assert_eq!(format_rfc3339(-1, 0), "1969-12-31T23:59:59Z");
    }

    #[test]
    fn go_json_escaping() {
        let mut w = String::new();
        json_string(&mut w, "a<b>&\"\u{1}\u{8}");
        assert_eq!(w, "\"a\\u003cb\\u003e\\u0026\\\"\\u0001\\b\"");
    }

    /// `json.Marshal` writes each invalid UTF-8 byte as `�`; a valid
    /// char of the Go form's reserved set is written as itself.
    #[test]
    fn go_json_invalid_utf8() {
        let mut w = String::new();
        json_string(&mut w, &gostr::from_go_bytes(b"a\xe2\x82b\xf4\x8f\xbf\xbf"));
        assert_eq!(
            gostr::to_go_bytes(&w).as_ref(),
            b"\"a\\ufffd\\ufffdb\xf4\x8f\xbf\xbf\""
        );
        // A message string field with invalid UTF-8 decodes (pulsar types
        // keep the bytes) and is written the same way.
        let msg = celestia_proto::cosmos::bank::v1beta1::MsgSend {
            from_address: gostr::from_go_bytes(b"x\xff").into_owned(),
            ..Default::default()
        };
        let enc = Encoder::new().unwrap_or_else(|e| unreachable!("{e}"));
        let mut w = String::new();
        enc.marshal_any(
            "/cosmos.bank.v1beta1.MsgSend",
            &prost::Message::encode_to_vec(&msg),
            &mut w,
        )
        .unwrap_or_else(|e| unreachable!("{e}"));
        assert!(w.contains("\"from_address\":\"x\\ufffd\""), "{w}");
    }
}
