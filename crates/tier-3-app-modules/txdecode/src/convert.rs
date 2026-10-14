//! Converts decoded [`Dyn`] trees of `TxBody` / `AuthInfo` into the prost
//! types, mirroring the Go struct contents (strings made valid UTF-8 lossily,
//! nil `math.Int` amounts as empty strings). Done by hand rather than with
//! prost decoding because Go accepts nesting (e.g. multisig `ModeInfo`) far
//! beyond prost's recursion limit.

use celestia_proto::Any;
use celestia_proto::cosmos::base::v1beta1::Coin;
use celestia_proto::cosmos::crypto::multisig::v1beta1::CompactBitArray;
use celestia_proto::cosmos::tx::v1beta1::{
    AuthInfo, Fee, ModeInfo, SignerInfo, Tip, TxBody, mode_info,
};

use crate::gogo::{Dyn, Val};

fn num(d: &Dyn, n: u32) -> u64 {
    match d.fields.get(&n) {
        Some(Val::Num(v)) => *v,
        _ => 0,
    }
}

fn string(d: &Dyn, n: u32) -> String {
    String::from_utf8_lossy(d.bytes(n)).into_owned()
}

fn msg(d: &Dyn, n: u32) -> Option<&Dyn> {
    match d.fields.get(&n) {
        Some(Val::Msg(m)) => Some(m),
        _ => None,
    }
}

fn any(d: &Dyn) -> Any {
    Any {
        type_url: string(d, 1),
        value: d.bytes(2).to_vec(),
    }
}

fn coin(d: &Dyn) -> Coin {
    Coin {
        denom: string(d, 1),
        amount: match d.fields.get(&2) {
            Some(Val::Custom(Some(b))) => String::from_utf8_lossy(b).into_owned(),
            _ => String::new(),
        },
    }
}

/// `TxBody` with `Any` values as received.
pub(crate) fn tx_body(d: &Dyn) -> TxBody {
    TxBody {
        messages: d.msg_list(1).into_iter().map(any).collect(),
        memo: string(d, 2),
        timeout_height: num(d, 3),
        extension_options: d.msg_list(1023).into_iter().map(any).collect(),
        non_critical_extension_options: d.msg_list(2047).into_iter().map(any).collect(),
    }
}

fn mode_info(d: &Dyn) -> ModeInfo {
    let sum = match d.oneofs.values().next() {
        Some(1) => msg(d, 1).map(|s| {
            mode_info::Sum::Single(mode_info::Single {
                mode: num(s, 1) as i32,
            })
        }),
        Some(2) => msg(d, 2).map(|m| {
            mode_info::Sum::Multi(mode_info::Multi {
                bitarray: msg(m, 1).map(|b| CompactBitArray {
                    extra_bits_stored: num(b, 1) as u32,
                    elems: b.bytes(2).to_vec(),
                }),
                mode_infos: m.msg_list(2).into_iter().map(mode_info).collect(),
            })
        }),
        _ => None,
    };
    ModeInfo { sum }
}

/// `AuthInfo`.
pub(crate) fn auth_info(d: &Dyn) -> AuthInfo {
    #[allow(deprecated)]
    AuthInfo {
        signer_infos: d
            .msg_list(1)
            .into_iter()
            .map(|s| SignerInfo {
                public_key: msg(s, 1).map(any),
                mode_info: msg(s, 2).map(mode_info),
                sequence: num(s, 3),
            })
            .collect(),
        fee: msg(d, 2).map(|f| Fee {
            amount: f.msg_list(1).into_iter().map(coin).collect(),
            gas_limit: num(f, 2),
            payer: string(f, 3),
            granter: string(f, 4),
        }),
        tip: msg(d, 3).map(|t| Tip {
            amount: t.msg_list(1).into_iter().map(coin).collect(),
            tipper: string(t, 2),
        }),
    }
}
