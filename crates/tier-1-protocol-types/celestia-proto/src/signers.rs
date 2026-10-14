//! Message signers from the `cosmos.msg.v1.signer` proto option
//! (`codec.GetMsgV2Signers`).

/// A signer field value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signer {
    /// A bech32 address string (account or validator prefix).
    Str(String),
    /// Raw address bytes.
    Raw(Vec<u8>),
}

include!("gen/signers.rs");

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn bank_send_and_multisend() {
        let m = crate::cosmos::bank::v1beta1::MsgSend {
            from_address: "a".into(),
            ..Default::default()
        };
        let s = get_signers("/cosmos.bank.v1beta1.MsgSend", &m.encode_to_vec())
            .unwrap()
            .unwrap();
        assert_eq!(s, vec![Signer::Str("a".into())]);
        let mm = crate::cosmos::bank::v1beta1::MsgMultiSend {
            inputs: vec![crate::cosmos::bank::v1beta1::Input {
                address: "x".into(),
                coins: vec![],
            }],
            outputs: vec![],
        };
        let s = get_signers("/cosmos.bank.v1beta1.MsgMultiSend", &mm.encode_to_vec())
            .unwrap()
            .unwrap();
        assert_eq!(s, vec![Signer::Str("x".into())]);
        assert!(get_signers("/cosmos.bank.v1beta1.Params", &[]).is_none());
    }
}
