//! The `sdk.Tx` interfaces the ante decorators use (cosmos-sdk fork
//! `x/auth/tx/builder.go` wrapper, `types/tx/types.go`, `x/auth/tx/sigs.go`),
//! including the places where Go panics on malformed input.

use celestia_proto::Any;
use celestia_proto::cosmos::crypto::multisig::v1beta1::CompactBitArray;
use celestia_proto::cosmos::tx::v1beta1::{Fee, ModeInfo, mode_info};
use celestia_proto::signers::Signer;
use sdk::SdkError;
use sdk::error::sdk as errs;
use sdk::tx::Tx;

use crate::pubkey::PubKey;
use crate::{Fail, R, idx, internal, panic};

/// Bech32 prefix of account addresses.
pub const ACCOUNT_PREFIX: &str = "celestia";
/// Bech32 prefix of validator operator addresses.
pub const VALIDATOR_PREFIX: &str = "celestiavaloper";
/// `address.MaxAddrLen`.
pub const MAX_ADDR_LEN: usize = 255;

/// `tx.AuthInfo.Fee` (a nil pointer dereference in Go when absent).
pub fn fee(tx: &Tx) -> R<&Fee> {
    tx.auth_info.fee.as_ref().map_or_else(
        || panic("runtime error: invalid memory address or nil pointer dereference"),
        Ok,
    )
}

/// `GetGas`.
pub fn gas(tx: &Tx) -> R<u64> {
    Ok(fee(tx)?.gas_limit)
}

/// `address.Bech32Codec.StringToBytes`.
pub fn bech32_to_bytes(text: &str, prefix: &str) -> Result<Vec<u8>, SdkError> {
    if text.trim().is_empty() {
        return Err(internal("empty address string is not allowed"));
    }
    let (hrp, bz) = encoding::bech32::decode_and_convert(text).map_err(internal)?;
    if hrp != prefix {
        return Err(errs::logic(format!(
            "hrp does not match bech32 prefix: expected '{prefix}' got '{hrp}'"
        )));
    }
    verify_address_format(&bz)?;
    Ok(bz)
}

/// `sdk.VerifyAddressFormat`.
pub fn verify_address_format(bz: &[u8]) -> Result<(), SdkError> {
    if bz.is_empty() {
        return Err(errs::unknown_address("addresses cannot be empty"));
    }
    if bz.len() > MAX_ADDR_LEN {
        return Err(errs::unknown_address(format!(
            "address max length is {MAX_ADDR_LEN}, got {}",
            bz.len()
        )));
    }
    Ok(())
}

/// `sdk.AccAddressFromBech32`.
pub fn acc_address_from_bech32(text: &str) -> Result<Vec<u8>, SdkError> {
    if text.trim().is_empty() {
        return Err(internal("empty address string is not allowed"));
    }
    let (hrp, bz) = encoding::bech32::decode_and_convert(text).map_err(internal)?;
    if hrp != ACCOUNT_PREFIX {
        return Err(internal(format!(
            "invalid Bech32 prefix; expected {ACCOUNT_PREFIX}, got {hrp}"
        )));
    }
    verify_address_format(&bz)?;
    Ok(bz)
}

/// `Bech32Codec.BytesToString` for account addresses.
#[must_use]
pub fn bytes_to_bech32(bz: &[u8]) -> String {
    if bz.is_empty() {
        return String::new();
    }
    encoding::bech32::encode(ACCOUNT_PREFIX, bz).unwrap_or_default()
}

/// Signers of one message (`codec.GetMsgAnySigners` via `x/tx` signing
/// context).
pub fn msg_signers(any: &Any) -> Result<Vec<Vec<u8>>, SdkError> {
    let Some(res) = celestia_proto::signers::get_signers(&any.type_url, &any.value) else {
        return Err(internal(format!(
            "no cosmos.msg.v1.signer option found for message {}; use DefineCustomGetSigners to specify a custom getter",
            any.type_url.trim_start_matches('/')
        )));
    };
    let signers = res.map_err(|e| internal(e.to_string()))?;
    signer_bytes(&any.type_url, signers)
}

/// The signing context's address decoding of a message's signer fields:
/// the validator address codec for `cosmos.ValidatorAddressString`
/// signer fields, the account codec otherwise (`Bech32Codec.StringToBytes`:
/// a wrong prefix is `ErrLogic`, decoding errors plain errors).
pub fn signer_bytes(type_url: &str, signers: Vec<Signer>) -> Result<Vec<Vec<u8>>, SdkError> {
    let prefix = if crate::signing::is_validator_signer(type_url) {
        VALIDATOR_PREFIX
    } else {
        ACCOUNT_PREFIX
    };
    signers
        .into_iter()
        .map(|s| match s {
            Signer::Str(s) => bech32_to_bytes(&s, prefix),
            Signer::Raw(b) => Ok(b),
        })
        .collect()
}

/// Signers of the tx's `i`-th message as `Tx.GetSigners` computes them
/// (cosmos-proto `anyutil.Unpack`, then the signing context). A type with
/// a registered protobuf-go (pulsar) Go type is decoded with its generated
/// unmarshal, which keeps invalid UTF-8 in strings like gogoproto; any
/// other type with `dynamicpb`, which rejects invalid UTF-8 only in the
/// string fields whose resolved descriptor is proto3
/// ([`crate::getsigners_schema`]). The signers are read from the
/// normalized message (the decoded gogoproto struct; strings in Go form,
/// `gostr`) after `dynamicpb`'s check on the bytes as received.
pub fn tx_msg_signers(tx: &Tx, i: usize) -> Result<Vec<Vec<u8>>, SdkError> {
    let raw = &tx.body.messages[i];
    celestia_proto::signers::unpack_check(&raw.type_url, &raw.value).map_err(internal)?;
    match tx.normalized_msgs.get(i) {
        Some(n) if n.type_url == raw.type_url => msg_signers(n),
        _ => msg_signers(raw),
    }
}

/// `Tx.GetSigners`: unique message signers in order, then the fee payer.
pub fn signers(tx: &Tx) -> Result<Vec<Vec<u8>>, SdkError> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    for i in 0..tx.msgs().len() {
        for s in tx_msg_signers(tx, i)? {
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    let payer = tx.auth_info.fee.as_ref().map_or("", |f| f.payer.as_str());
    if !payer.is_empty() {
        let addr = bech32_to_bytes(payer, ACCOUNT_PREFIX)?;
        if !out.contains(&addr) {
            out.push(addr);
        }
    }
    Ok(out)
}

/// `wrapper.FeePayer`.
pub fn fee_payer(tx: &Tx) -> R<Vec<u8>> {
    let f = fee(tx)?;
    if !f.payer.is_empty() {
        return bech32_to_bytes(&f.payer, ACCOUNT_PREFIX).or_else(|e| panic(e.log));
    }
    match signers(tx) {
        Ok(s) => Ok(idx(&s, 0)?.clone()),
        // GetSigners error → nil fee payer.
        Err(_) => Ok(Vec::new()),
    }
}

/// `wrapper.FeeGranter`.
pub fn fee_granter(tx: &Tx) -> R<Option<Vec<u8>>> {
    let f = fee(tx)?;
    if f.granter.is_empty() {
        return Ok(None);
    }
    bech32_to_bytes(&f.granter, ACCOUNT_PREFIX).map_or_else(|e| panic(e.log), |b| Ok(Some(b)))
}

/// `wrapper.GetPubKeys`.
pub fn pub_keys(tx: &Tx) -> R<Vec<Option<PubKey>>> {
    tx.auth_info
        .signer_infos
        .iter()
        .map(|si| match &si.public_key {
            None => Ok(None),
            Some(any) => PubKey::from_any(any)
                .map(Some)
                .map_err(|e| Fail::Err(errs::logic(format!("Expecting PubKey, got: {}", e.log)))),
        })
        .collect()
}

/// `signing.SignatureData`.
#[derive(Debug, Clone, PartialEq)]
pub enum SigData {
    /// `SingleSignatureData`.
    Single {
        /// Sign mode (`signing.SignMode`).
        mode: i32,
        /// Raw signature.
        signature: Vec<u8>,
    },
    /// `MultiSignatureData`.
    Multi {
        /// Signer bit array (may be nil).
        bitarray: Option<CompactBitArray>,
        /// Sub-signatures: one per `MultiSignature` entry, `None` (Go's nil
        /// interface) past the mode infos.
        signatures: Vec<Option<SigData>>,
    },
}

/// `signing.SignatureV2`.
#[derive(Debug, Clone, PartialEq)]
pub struct SignatureV2 {
    /// Public key from the signer info.
    pub pub_key: Option<PubKey>,
    /// Signature data (`nil` when the signer info has no mode info).
    pub data: Option<SigData>,
    /// Signer sequence.
    pub sequence: u64,
}

/// `decodeMultisignatures`: a `MultiSignature` with no unknown fields.
pub fn decode_multisignatures(bz: &[u8]) -> Result<Vec<Vec<u8>>, SdkError> {
    let mut out = Vec::new();
    let mut buf = bz;
    let mut unknown = false;
    while !buf.is_empty() {
        let (key, n) = varint(buf).ok_or_else(|| internal("unexpected EOF"))?;
        buf = &buf[n..];
        // gogoproto: `int32(wire >> 3)`, so keys past `u32::MAX` truncate
        // (a field number of 1 in the low 32 bits is a signature).
        let field = (key >> 3) as u32 as i32;
        let wire = key & 7;
        if wire == 4 {
            return Err(internal(
                "proto: MultiSignature: wiretype end group for non-group",
            ));
        }
        if field <= 0 {
            return Err(internal(format!(
                "proto: MultiSignature: illegal tag {field} (wire type {key})"
            )));
        }
        if field == 1 {
            if wire != 2 {
                return Err(internal(format!(
                    "proto: wrong wireType = {wire} for field Signatures"
                )));
            }
            let (len, n) = varint(buf).ok_or_else(|| internal("unexpected EOF"))?;
            buf = &buf[n..];
            let len = usize::try_from(len).map_err(|_| internal("invalid length"))?;
            if len > buf.len() {
                return Err(internal("unexpected EOF"));
            }
            out.push(buf[..len].to_vec());
            buf = &buf[len..];
        } else {
            unknown = true;
            buf = skip_field(buf, wire)?;
        }
    }
    if unknown {
        return Err(internal(
            "rejecting unrecognized fields found in MultiSignature",
        ));
    }
    Ok(out)
}

fn varint(b: &[u8]) -> Option<(u64, usize)> {
    let mut v: u64 = 0;
    for (i, byte) in b.iter().enumerate().take(10) {
        v |= u64::from(byte & 0x7f) << (7 * i);
        if byte & 0x80 == 0 {
            return Some((v, i + 1));
        }
    }
    None
}

fn skip_field(buf: &[u8], wire: u64) -> Result<&[u8], SdkError> {
    let eof = || internal("unexpected EOF");
    match wire {
        0 => {
            let (_, n) = varint(buf).ok_or_else(eof)?;
            Ok(&buf[n..])
        }
        1 => buf.get(8..).ok_or_else(eof),
        2 => {
            let (len, n) = varint(buf).ok_or_else(eof)?;
            let len = usize::try_from(len).map_err(|_| eof())?;
            buf.get(n..).and_then(|b| b.get(len..)).ok_or_else(eof)
        }
        5 => buf.get(4..).ok_or_else(eof),
        w => Err(internal(format!("proto: illegal wireType {w}"))),
    }
}

/// `ModeInfoAndSigToSignatureData`.
pub fn mode_info_to_sig_data(mi: &ModeInfo, sig: &[u8]) -> R<SigData> {
    match &mi.sum {
        Some(mode_info::Sum::Single(s)) => Ok(SigData::Single {
            mode: s.mode,
            signature: sig.to_vec(),
        }),
        Some(mode_info::Sum::Multi(m)) => {
            let sigs = decode_multisignatures(sig)?;
            // `make([]signing.SignatureData, len(sigs))`, filled for the
            // mode infos (more mode infos than signatures: index panic).
            let mut out = vec![None; sigs.len()];
            for (i, mi) in m.mode_infos.iter().enumerate() {
                let d = mode_info_to_sig_data(mi, idx(&sigs, i)?)?;
                out[i] = Some(d);
            }
            Ok(SigData::Multi {
                bitarray: m.bitarray.clone(),
                signatures: out,
            })
        }
        None => panic("unexpected ModeInfo data type <nil>"),
    }
}

/// `wrapper.GetSignaturesV2`.
pub fn signatures_v2(tx: &Tx) -> R<Vec<SignatureV2>> {
    let pks = pub_keys(tx)?;
    let mut out = Vec::with_capacity(tx.auth_info.signer_infos.len());
    for (i, si) in tx.auth_info.signer_infos.iter().enumerate() {
        let pub_key = idx(&pks, i)?.clone();
        match &si.mode_info {
            None => out.push(SignatureV2 {
                pub_key,
                data: None,
                sequence: 0,
            }),
            Some(mi) => {
                let data = mode_info_to_sig_data(mi, idx(&tx.signatures, i)?)?;
                out.push(SignatureV2 {
                    pub_key,
                    data: Some(data),
                    sequence: si.sequence,
                });
            }
        }
    }
    Ok(out)
}

/// `CompactBitArray.Count` (nil → 0; may be negative for malformed input).
#[must_use]
pub fn bit_count(ba: Option<&CompactBitArray>) -> i64 {
    match ba {
        None => 0,
        Some(b) if b.extra_bits_stored == 0 => (b.elems.len() as i64) * 8,
        Some(b) => (b.elems.len() as i64 - 1) * 8 + i64::from(b.extra_bits_stored),
    }
}

/// `CompactBitArray.GetIndex`.
pub fn bit_get(ba: Option<&CompactBitArray>, i: i64) -> R<bool> {
    let Some(b) = ba else {
        return Ok(false);
    };
    if i < 0 || i >= bit_count(ba) {
        return Ok(false);
    }
    let byte = *idx(&b.elems, usize::try_from(i >> 3).unwrap_or(usize::MAX))?;
    let shift = 7 - (i % 8);
    Ok(byte & (1u8 << shift) > 0)
}

/// `CompactBitArray.NumTrueBitsBefore`.
pub fn bit_num_true_before(ba: Option<&CompactBitArray>, index: i64) -> R<i64> {
    let Some(b) = ba else {
        return panic("runtime error: invalid memory address or nil pointer dereference");
    };
    let max = bit_count(ba);
    let index = index.min(max);
    let mut ones = 0i64;
    for (elem, byte) in b.elems.iter().enumerate() {
        let elem = elem as i64;
        if elem * 8 + 7 >= index {
            // Go: shift by (7 - index%8 + 1); a shift >= 8 yields 0. A
            // negative index makes Go's `%` negative and the shift > 8.
            let shift = 7 - (index % 8) + 1;
            let v = if (0..8).contains(&shift) {
                byte >> shift
            } else if shift < 0 {
                return panic("runtime error: negative shift amount");
            } else {
                0
            };
            ones += i64::from(v.count_ones());
            return Ok(ones);
        }
        ones += i64::from(byte.count_ones());
    }
    Ok(ones)
}

/// `any.unpack::<M>()` as Go holds the message (gogoproto's decoding, see
/// `txdecode::prost_compatible`): e.g. a key past `u32::MAX` is an unknown
/// field to Go, where prost fails.
fn unpack_go<M: prost::Message + prost::Name + Default>(
    any: &Any,
) -> Option<Result<M, prost::DecodeError>> {
    if any.type_url != M::type_url() {
        return None;
    }
    let fixed = txdecode::prost_compatible(any);
    fixed.as_ref().unwrap_or(any).unpack::<M>()
}

/// Unpacks `authz.MsgExec.Msgs`.
pub fn msg_exec_msgs(any: &Any) -> R<Option<Vec<Any>>> {
    use celestia_proto::cosmos::authz::v1beta1::MsgExec;
    match unpack_go::<MsgExec>(any) {
        Some(Ok(m)) => Ok(Some(m.msgs)),
        Some(Err(e)) => Err(Fail::Err(errs::tx_decode(e.to_string()))),
        None => Ok(None),
    }
}

/// Unpacks `govv1.MsgSubmitProposal.Messages`.
pub fn submit_proposal_msgs(any: &Any) -> R<Option<Vec<Any>>> {
    use celestia_proto::cosmos::gov::v1::MsgSubmitProposal;
    match unpack_go::<MsgSubmitProposal>(any) {
        Some(Ok(m)) => Ok(Some(m.messages)),
        Some(Err(e)) => Err(Fail::Err(errs::tx_decode(e.to_string()))),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use celestia_proto::cosmos::crypto::multisig::v1beta1::MultiSignature;
    use prost::Message as _;

    use super::*;

    fn single() -> ModeInfo {
        ModeInfo {
            sum: Some(mode_info::Sum::Single(mode_info::Single { mode: 1 })),
        }
    }

    fn multi(mode_infos: Vec<ModeInfo>) -> ModeInfo {
        ModeInfo {
            sum: Some(mode_info::Sum::Multi(mode_info::Multi {
                bitarray: None,
                mode_infos,
            })),
        }
    }

    #[test]
    fn multisig_entries_past_the_mode_infos_are_nil() {
        // Go: `make([]SignatureData, len(sigs))` filled per mode info.
        let sig = MultiSignature {
            signatures: vec![vec![1; 64], vec![2; 64]],
        }
        .encode_to_vec();
        let d = mode_info_to_sig_data(&multi(vec![single()]), &sig).unwrap();
        let SigData::Multi { signatures, .. } = d else {
            unreachable!()
        };
        assert_eq!(signatures.len(), 2);
        assert!(signatures[0].is_some() && signatures[1].is_none());
        // More mode infos than signatures: index out of range.
        let r = mode_info_to_sig_data(&multi(vec![single(), single(), single()]), &sig);
        assert!(matches!(r, Err(Fail::Panic(_))));
    }

    /// Key 17179869176: gogoproto's field `int32(key >> 3)` is 2^31 - 1, an
    /// unknown field it skips; prost rejects keys past `u32::MAX`.
    const WIDE_KEY_VARINT: [u8; 6] = [0xf8, 0xff, 0xff, 0xff, 0x3f, 0x00];

    #[test]
    fn nested_msgs_are_read_as_gogoproto_decodes_them() {
        use celestia_proto::cosmos::authz::v1beta1::MsgExec;
        use celestia_proto::cosmos::gov::v1::MsgSubmitProposal;
        let inner = Any {
            type_url: "/cosmos.staking.v1beta1.MsgUndelegate".into(),
            value: Vec::new(),
        };
        let mut v = MsgSubmitProposal {
            messages: vec![inner.clone()],
            ..Default::default()
        }
        .encode_to_vec();
        v.extend_from_slice(&WIDE_KEY_VARINT);
        let prop = Any {
            type_url: "/cosmos.gov.v1.MsgSubmitProposal".into(),
            value: v,
        };
        assert!(MsgSubmitProposal::decode(prop.value.as_slice()).is_err());
        assert_eq!(
            submit_proposal_msgs(&prop).unwrap(),
            Some(vec![inner.clone()])
        );
        let mut v = MsgExec {
            grantee: "g".into(),
            msgs: vec![inner.clone()],
        }
        .encode_to_vec();
        v.extend_from_slice(&WIDE_KEY_VARINT);
        let exec = Any {
            type_url: "/cosmos.authz.v1beta1.MsgExec".into(),
            value: v,
        };
        assert_eq!(msg_exec_msgs(&exec).unwrap(), Some(vec![inner]));
    }

    #[test]
    fn multisignature_keys_truncate_to_int32() {
        // Key (1 << 35) | (1 << 3) | 2: field `int32(key >> 3)` = 1.
        let mut sig = vec![0x8a, 0x80, 0x80, 0x80, 0x80, 0x01];
        sig.extend_from_slice(&[0x02, 0xaa, 0xbb]);
        assert_eq!(
            decode_multisignatures(&sig).unwrap(),
            vec![vec![0xaa, 0xbb]]
        );
        // Field 2^31 - 1: unknown, rejected.
        let r = decode_multisignatures(&WIDE_KEY_VARINT);
        assert_eq!(
            r.unwrap_err().log,
            "rejecting unrecognized fields found in MultiSignature"
        );
        // `int32(key >> 3)` negative: illegal tag.
        let r = decode_multisignatures(&[0x80, 0x80, 0x80, 0x80, 0x40, 0x00]);
        assert_eq!(
            r.unwrap_err().log,
            "proto: MultiSignature: illegal tag -2147483648 (wire type 17179869184)"
        );
        // End group.
        let r = decode_multisignatures(&[0x0c]);
        assert_eq!(
            r.unwrap_err().log,
            "proto: MultiSignature: wiretype end group for non-group"
        );
    }
}
