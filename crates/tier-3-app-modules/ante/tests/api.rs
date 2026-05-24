//! API tests for the M3 ante crate.

#![allow(clippy::expect_used)]

use ante::{AnteConfig, AnteHandler, Error, Gas, GasMeter, ModeInfo, SignMode, SignerInfo, TxRaw};
use cosmos_types::{Any, AuthInfo, Fee, TxBody};
use prost::Message;

#[test]
fn gas_meter_records_out_of_gas_and_overflow() {
    let mut meter = GasMeter::new(5);
    assert_eq!(meter.remaining(), Some(5));
    assert_eq!(meter.consume(3, "read"), Ok(()));
    assert_eq!(meter.remaining(), Some(2));
    assert_eq!(meter.consume(3, "write"), Err(Error::OutOfGas));
    assert_eq!(meter.consumed, 6);
    assert!(meter.is_out_of_gas());
    assert_eq!(meter.remaining(), Some(0));

    let mut infinite = GasMeter::infinite();
    assert_eq!(infinite.consume(Gas::MAX, "large"), Ok(()));
    assert_eq!(infinite.remaining(), None);
    assert!(!infinite.is_out_of_gas());

    let mut overflow = GasMeter {
        limit: Gas::MAX - 1,
        consumed: Gas::MAX,
    };
    assert_eq!(overflow.consume(1, "overflow"), Err(Error::GasOverflow));
}

#[test]
fn stateless_validation_accepts_direct_single_signer_tx() {
    let handler = handler();
    let tx_bytes = tx_bytes(SignMode::Direct, vec![message_any()], "", Vec::new(), 1);
    let tx = handler
        .validate_tx_stateless(&tx_bytes)
        .expect("valid direct tx");

    assert_eq!(tx.signatures, vec![vec![0xab; 64]]);
}

#[test]
fn stateless_validation_rejects_empty_messages_extensions_and_long_memo() {
    let handler = handler();
    let empty_messages = tx_bytes(SignMode::Direct, Vec::new(), "memo", Vec::new(), 1);
    assert_eq!(
        handler.validate_tx_stateless(&empty_messages),
        Err(Error::MalformedTx)
    );

    let extension_options = tx_bytes(
        SignMode::Direct,
        vec![message_any()],
        "",
        vec![message_any()],
        1,
    );
    assert_eq!(
        handler.validate_tx_stateless(&extension_options),
        Err(Error::MalformedTx)
    );

    let long_memo = "x".repeat(257);
    let long_memo_tx = tx_bytes(
        SignMode::Direct,
        vec![message_any()],
        &long_memo,
        Vec::new(),
        1,
    );
    assert_eq!(
        handler.validate_tx_stateless(&long_memo_tx),
        Err(Error::InvalidMemo)
    );
}

#[test]
fn stateless_validation_rejects_unsupported_sign_modes() {
    let handler = handler();
    let amino_tx = tx_bytes(
        SignMode::LegacyAminoJson,
        vec![message_any()],
        "",
        Vec::new(),
        1,
    );
    assert_eq!(
        handler.validate_tx_stateless(&amino_tx),
        Err(Error::UnsupportedSignMode)
    );
}

fn handler() -> AnteHandler {
    AnteHandler::new(AnteConfig::celestia_mainnet_app_v6("celestia"))
}

fn tx_bytes(
    sign_mode: SignMode,
    messages: Vec<Any>,
    memo: &str,
    extension_options: Vec<Any>,
    sequence: u64,
) -> Vec<u8> {
    TxRaw {
        body_bytes: TxBody {
            messages,
            memo: memo.to_owned(),
            timeout_height: 0,
            extension_options,
            non_critical_extension_options: Vec::new(),
        }
        .to_proto()
        .encode_to_vec(),
        auth_info_bytes: AuthInfo {
            signer_infos: vec![SignerInfo {
                public_key: None,
                mode_info: ModeInfo::Single { mode: sign_mode },
                sequence,
            }],
            fee: Some(Fee {
                amount: Vec::new(),
                gas_limit: 200_000,
                payer: String::new(),
                granter: String::new(),
            }),
            tip: None,
        }
        .to_proto()
        .encode_to_vec(),
        signatures: vec![vec![0xab; 64]],
    }
    .encode_to_vec()
    .expect("valid tx raw")
}

fn message_any() -> Any {
    Any {
        type_url: "/test.Msg".to_owned(),
        value: vec![1, 2, 3],
    }
}
