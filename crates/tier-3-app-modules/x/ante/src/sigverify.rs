//! Signature decorators (cosmos-sdk `x/auth/ante/sigverify.go`).

use base64::Engine as _;
use prost::Message;
use sdk::Context;
use sdk::error::sdk as errs;
use sdk::event::event;
use sdk::tx::Tx;
use x_auth::Account;

use crate::deps::AnteDeps;
use crate::pubkey::PubKey;
use crate::signing::{self, SignerData};
use crate::txview::{self, SigData};
use crate::{Fail, R, consume_gas, idx, internal, is_recheck_tx};

/// `GetSignerAcc`.
fn get_signer_acc(ctx: &mut Context<'_>, addr: &[u8]) -> R<Account> {
    x_auth::get_account(ctx, addr)?.map_or_else(
        || {
            Err(errs::unknown_address(format!(
                "account {} does not exist",
                txview::bytes_to_bech32(addr)
            ))
            .into())
        },
        Ok,
    )
}

/// The account's public key (`acc.GetPubKey()`).
fn account_pub_key(acc: &Account) -> R<Option<PubKey>> {
    match acc.pub_key() {
        None => Ok(None),
        Some(any) => PubKey::from_any(any)
            .map(Some)
            .or_else(|e| crate::panic(e.log)),
    }
}

/// `signatureDataToBz`.
fn signature_data_to_bz(data: Option<&SigData>) -> R<Vec<Vec<u8>>> {
    match data {
        None => Err(internal("got empty SignatureData").into()),
        Some(SigData::Single { signature, .. }) => Ok(vec![signature.clone()]),
        Some(SigData::Multi { signatures, .. }) => {
            let mut sigs = Vec::new();
            for d in signatures {
                sigs.extend(signature_data_to_bz(Some(d))?);
            }
            let agg = celestia_proto::cosmos::crypto::multisig::v1beta1::MultiSignature {
                signatures: sigs.clone(),
            }
            .encode_to_vec();
            sigs.push(agg);
            Ok(sigs)
        }
    }
}

/// `SetPubKeyDecorator`: stores signer-info public keys on accounts that
/// have none and emits the `acc_seq` / `signature` tx events.
pub fn set_pub_key(ctx: &mut Context<'_>, tx: &Tx, simulate: bool, deps: &impl AnteDeps) -> R<()> {
    let pubkeys = txview::pub_keys(tx)?;
    let signers = txview::signers(tx).map_err(Fail::ZeroCtx)?;
    let mut signer_strs = vec![String::new(); signers.len()];
    for (i, pk) in pubkeys.iter().enumerate() {
        let signer = idx(&signers, i)?.clone();
        signer_strs[i] = txview::bytes_to_bech32(&signer);
        let pk = match pk {
            Some(pk) => pk.clone(),
            None if !simulate => continue,
            None => PubKey::sim(),
        };
        if !simulate && pk.address()? != signer && deps.sigverify_tx() {
            return Err(errs::invalid_pub_key(format!(
                "pubKey does not match signer address {} with signer index: {i}",
                signer_strs[i]
            ))
            .into());
        }
        let mut acc = get_signer_acc(ctx, &signer)?;
        if acc.pub_key().is_some() {
            continue;
        }
        if let Account::Module(_) = acc {
            return Err(errs::invalid_pub_key("not supported for module accounts").into());
        }
        acc.base_mut().pub_key = Some(pk.to_any());
        x_auth::set_account(ctx, &acc)?;
    }

    let sigs = txview::signatures_v2(tx)?;
    let mut events = Vec::new();
    for (i, sig) in sigs.iter().enumerate() {
        let signer_str = idx(&signer_strs, i)?;
        events.push(event(
            "tx",
            [("acc_seq", format!("{signer_str}/{}", sig.sequence))],
        ));
        for bz in signature_data_to_bz(sig.data.as_ref())? {
            events.push(event(
                "tx",
                [(
                    "signature",
                    base64::engine::general_purpose::STANDARD.encode(bz),
                )],
            ));
        }
    }
    ctx.events.emit_all(events);
    Ok(())
}

/// `ValidateSigCountDecorator`.
pub fn validate_sig_count(ctx: &mut Context<'_>, tx: &Tx) -> R<()> {
    let params = x_auth::get_params(ctx)?;
    let pubkeys = txview::pub_keys(tx)?;
    let mut count: u64 = 0;
    for pk in pubkeys.iter().flatten() {
        count += pk.count_sub_keys();
        if count > params.tx_sig_limit {
            return Err(errs::too_many_signatures(format!(
                "signatures: {count}, limit: {}",
                params.tx_sig_limit
            ))
            .into());
        }
    }
    Ok(())
}

/// `DefaultSigVerificationGasConsumer`.
fn consume_sig_gas(
    ctx: &mut Context<'_>,
    pk: Option<&PubKey>,
    data: Option<&SigData>,
    params: &celestia_proto::cosmos::auth::v1beta1::Params,
) -> R<()> {
    match pk {
        Some(PubKey::Ed25519(_)) => {
            consume_gas(ctx, params.sig_verify_cost_ed25519, "ante verify: ed25519")?;
            Err(errs::invalid_pub_key("ED25519 public keys are unsupported").into())
        }
        Some(PubKey::Secp256k1(_)) => consume_gas(
            ctx,
            params.sig_verify_cost_secp256k1,
            "ante verify: secp256k1",
        ),
        Some(PubKey::Secp256r1(_)) => consume_gas(
            ctx,
            params.sig_verify_cost_secp256k1 / 2,
            "ante verify: secp256r1",
        ),
        Some(PubKey::Multisig { keys, .. }) => {
            let Some(SigData::Multi {
                bitarray,
                signatures,
            }) = data
            else {
                return Err(internal(format!(
                    "expected *signing.MultiSignatureData, got, {}",
                    match data {
                        Some(SigData::Single { .. }) => "*signing.SingleSignatureData",
                        _ => "<nil>",
                    }
                ))
                .into());
            };
            // ConsumeMultisignatureVerificationGas
            let ba = bitarray.as_ref();
            let size = txview::bit_count(ba);
            let mut sig_index = 0usize;
            for i in 0..size {
                if !txview::bit_get(ba, i)? {
                    continue;
                }
                let sub_pk = idx(keys, usize::try_from(i).unwrap_or(usize::MAX))?.clone();
                let sub_sig = idx(signatures, sig_index)?.clone();
                consume_sig_gas(ctx, Some(&sub_pk), Some(&sub_sig), params)?;
                sig_index += 1;
            }
            Ok(())
        }
        None => Err(errs::invalid_pub_key("unrecognized public key type: <nil>").into()),
    }
}

/// `SigGasConsumeDecorator`.
pub fn sig_gas_consume(ctx: &mut Context<'_>, tx: &Tx, simulate: bool) -> R<()> {
    let params = x_auth::get_params(ctx)?;
    let sigs = txview::signatures_v2(tx)?;
    let signers = txview::signers(tx)?;
    for (i, sig) in sigs.iter().enumerate() {
        let acc = get_signer_acc(ctx, idx(&signers, i)?)?;
        let mut pk = account_pub_key(&acc)?;
        if simulate && pk.is_none() {
            pk = Some(PubKey::sim());
        }
        consume_sig_gas(ctx, pk.as_ref(), sig.data.as_ref(), &params)?;
    }
    Ok(())
}

/// `OnlyLegacyAminoSigners`.
fn only_legacy_amino_signers(data: Option<&SigData>) -> bool {
    match data {
        Some(SigData::Single { mode, .. }) => *mode == signing::SIGN_MODE_LEGACY_AMINO_JSON,
        Some(SigData::Multi { signatures, .. }) => signatures
            .iter()
            .all(|s| only_legacy_amino_signers(Some(s))),
        None => false,
    }
}

/// `SigVerificationDecorator` (no verification on ReCheckTx or simulation).
pub fn sig_verification(
    ctx: &mut Context<'_>,
    tx: &Tx,
    simulate: bool,
    deps: &impl AnteDeps,
) -> R<()> {
    let sigs = txview::signatures_v2(tx)?;
    let signers = txview::signers(tx)?;
    if sigs.len() != signers.len() {
        return Err(errs::unauthorized(format!(
            "invalid number of signer;  expected: {}, got {}",
            signers.len(),
            sigs.len()
        ))
        .into());
    }
    for (i, sig) in sigs.iter().enumerate() {
        let acc = get_signer_acc(ctx, &signers[i])?;
        let pk = account_pub_key(&acc)?;
        if !simulate && pk.is_none() {
            return Err(errs::invalid_pub_key("pubkey on account is not set").into());
        }
        if sig.sequence != acc.sequence() {
            return Err(errs::wrong_sequence(format!(
                "account sequence mismatch, expected {}, got {}",
                acc.sequence(),
                sig.sequence
            ))
            .into());
        }
        let genesis = ctx.height() == 0;
        let chain_id = ctx.header.chain_id.clone();
        let acc_num = if genesis { 0 } else { acc.account_number() };
        if !simulate && !is_recheck_tx(ctx.mode) && deps.sigverify_tx() {
            let Some(pk) = pk else {
                return Err(errs::invalid_pub_key("pubkey on account is not set").into());
            };
            let sd = SignerData {
                address: acc.address_str().to_owned(),
                chain_id: chain_id.clone(),
                account_number: acc_num,
                sequence: acc.sequence(),
                pub_key: pk.to_any(),
            };
            signing::check_signing_tx_data(tx)?;
            if let Err(err) = signing::verify_signature(&pk, &sd, sig.data.as_ref(), tx)? {
                let msg = if only_legacy_amino_signers(sig.data.as_ref()) {
                    format!(
                        "signature verification failed; please verify account number ({acc_num}), sequence ({}) and chain-id ({chain_id})",
                        acc.sequence()
                    )
                } else {
                    format!(
                        "signature verification failed; please verify account number ({acc_num}) and chain-id ({chain_id}): ({err})"
                    )
                };
                return Err(errs::unauthorized(msg).into());
            }
        }
    }
    Ok(())
}

/// `IncrementSequenceDecorator`.
pub fn increment_sequence(ctx: &mut Context<'_>, tx: &Tx) -> R<()> {
    let signers = txview::signers(tx).map_err(Fail::ZeroCtx)?;
    for signer in &signers {
        let Some(mut acc) = x_auth::get_account(ctx, signer)? else {
            return crate::panic(
                "runtime error: invalid memory address or nil pointer dereference",
            );
        };
        let b = acc.base_mut();
        b.sequence = b.sequence.wrapping_add(1);
        x_auth::set_account(ctx, &acc)?;
    }
    Ok(())
}
