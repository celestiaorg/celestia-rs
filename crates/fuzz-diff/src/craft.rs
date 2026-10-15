//! Signed txs with invalid UTF-8 in string fields for the `utf8` devnet
//! workload (`conformance/devnet/workloads/utf8.sh`): Go celestia-appd
//! executes them on a real chain, and the recorded scenario is then
//! replayed on the Rust app (`replay_scenarios`). The keys are the devnet's
//! test keyring (`val0`, `user0`).
//!
//! Strings are built as Go forms (`gostr`), so prost encodes the raw bytes.

use celestia_proto::Any;
use celestia_proto::cosmos::authz::v1beta1 as authz;
use celestia_proto::cosmos::bank::v1beta1 as bank;
use celestia_proto::cosmos::base::v1beta1::Coin;
use celestia_proto::cosmos::feegrant::v1beta1 as feegrant;
use celestia_proto::cosmos::gov::{v1 as gov, v1beta1 as govb};
use celestia_proto::cosmos::staking::v1beta1 as staking;
use celestia_proto::cosmos::tx::signing::v1beta1::SignMode;
use celestia_proto::cosmos::tx::v1beta1::{
    AuthInfo, Fee, ModeInfo, SignDoc, SignerInfo, TxBody, TxRaw, mode_info,
};
use prost::Message as _;

/// The gov module account.
pub const GOV: &str = "celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7";

/// A Go-form string of the bytes `b`.
fn gs(b: &[u8]) -> String {
    gostr::from_go_bytes(b).into_owned()
}

fn utia(n: u64) -> Vec<Coin> {
    vec![Coin {
        denom: "utia".into(),
        amount: n.to_string(),
    }]
}

/// The devnet accounts.
pub struct Accounts {
    /// `val0` account address.
    pub val0: String,
    /// `val0` operator address.
    pub val0_oper: String,
    /// `user0` account address.
    pub user0: String,
}

/// One workload tx: signer key, sign mode, messages and memo.
pub struct Case {
    /// What it exercises.
    pub what: &'static str,
    /// Signer key name.
    pub signer: &'static str,
    /// `SIGN_MODE_LEGACY_AMINO_JSON` instead of `SIGN_MODE_DIRECT`.
    pub amino: bool,
    /// Messages.
    pub msgs: Vec<Any>,
    /// Memo (Go form).
    pub memo: String,
}

fn edit_validator(a: &Accounts, d: staking::Description) -> Any {
    Any::pack(&staking::MsgEditValidator {
        description: Some(d),
        validator_address: a.val0_oper.clone(),
        commission_rate: String::new(),
        min_self_delegation: String::new(),
    })
}

fn blob_params() -> Any {
    Any::pack(&celestia_proto::celestia::blob::v1::MsgUpdateBlobParams {
        authority: GOV.into(),
        params: Some(celestia_proto::celestia::blob::v1::Params {
            gas_per_blob_byte: 8,
            gov_max_square_size: 64,
        }),
    })
}

fn proposal(a: &Accounts, msgs: Vec<Any>, title: &[u8], summary: &[u8], meta: &[u8]) -> Any {
    Any::pack(&gov::MsgSubmitProposal {
        messages: msgs,
        initial_deposit: utia(10_000_000),
        proposer: a.user0.clone(),
        metadata: gs(meta),
        title: gs(title),
        summary: gs(summary),
        expedited: false,
    })
}

/// The workload's txs in order (`case` numbers are indexes). Proposal ids:
/// 1 (case 3), 2 (case 9), 3 (case 10); the gov genesis patch makes them
/// pass within the run.
#[must_use]
pub fn cases(a: &Accounts) -> Vec<Case> {
    let c = |what, signer, amino, msgs, memo: &[u8]| Case {
        what,
        signer,
        amino,
        msgs,
        memo: gs(memo),
    };
    // 70 Go bytes, 30 of them invalid: the moniker limit counts bytes.
    let mut moniker70 = b"m".repeat(40);
    moniker70.extend(std::iter::repeat_n(0xffu8, 30));
    let mut details281 = b"d".repeat(271);
    details281.extend(std::iter::repeat_n(0xfeu8, 10));
    vec![
        c(
            "edit-validator: invalid UTF-8 in every description field, memo",
            "val0",
            false,
            vec![edit_validator(
                a,
                staking::Description {
                    moniker: gs(b"val0-\xff\xfe-moniker"),
                    identity: gs(b"\xc0\xaf"),
                    website: gs(b"https://\xf4\x8f\xbf\xbf.example/\xed\xa0\x80"),
                    security_contact: gs(b"\xe2\x82"),
                    details: gs(b"details \x80\x81 <&>"),
                },
            )],
            b"memo \xc3",
        ),
        c(
            "edit-validator: moniker of 70 bytes with 30 invalid",
            "val0",
            false,
            vec![edit_validator(
                a,
                staking::Description {
                    moniker: gs(&moniker70),
                    identity: "[do-not-modify]".into(),
                    website: "[do-not-modify]".into(),
                    security_contact: "[do-not-modify]".into(),
                    details: "[do-not-modify]".into(),
                },
            )],
            b"",
        ),
        c(
            "edit-validator: details of 281 bytes (fails, length in the log)",
            "val0",
            false,
            vec![edit_validator(
                a,
                staking::Description {
                    moniker: "[do-not-modify]".into(),
                    identity: "[do-not-modify]".into(),
                    website: "[do-not-modify]".into(),
                    security_contact: "[do-not-modify]".into(),
                    details: gs(&details281),
                },
            )],
            b"",
        ),
        c(
            "gov v1 proposal 1: invalid title / summary / metadata",
            "user0",
            false,
            vec![proposal(
                a,
                vec![blob_params()],
                b"title \xfe",
                b"summary \x80 \xf4\x8f\xbe\x80",
                b"ipfs://\xff",
            )],
            b"",
        ),
        c(
            "gov v1 proposal: JSON metadata title (U+FFFD) differs from the raw title (fails)",
            "user0",
            false,
            vec![proposal(
                a,
                vec![blob_params()],
                b"t\xff",
                b"s",
                b"{\"title\":\"t\xff\",\"summary\":\"s\"}",
            )],
            b"",
        ),
        c(
            "gov vote with invalid metadata",
            "val0",
            false,
            vec![Any::pack(&gov::MsgVote {
                proposal_id: 1,
                voter: a.val0.clone(),
                option: gov::VoteOption::Yes as i32,
                metadata: gs(b"vote \xff"),
            })],
            b"",
        ),
        c(
            "gov weighted vote with invalid metadata",
            "user0",
            false,
            vec![Any::pack(&gov::MsgVoteWeighted {
                proposal_id: 1,
                voter: a.user0.clone(),
                options: vec![gov::WeightedVoteOption {
                    option: gov::VoteOption::Yes as i32,
                    weight: "1.000000000000000000".into(),
                }],
                metadata: gs(b"\xfe\xfd"),
            })],
            b"",
        ),
        c(
            "gov v1beta1 text proposal with invalid title / description",
            "user0",
            false,
            vec![Any::pack(&govb::MsgSubmitProposal {
                content: Some(Any::pack(&govb::TextProposal {
                    title: gs(b"legacy \xff"),
                    description: gs(b"desc \xe2\x82"),
                })),
                initial_deposit: utia(10_000_000),
                proposer: a.user0.clone(),
            })],
            b"",
        ),
        c(
            "authz grant: generic MsgEditValidator and MsgVote to user0",
            "val0",
            false,
            vec![
                Any::pack(&authz::MsgGrant {
                    granter: a.val0.clone(),
                    grantee: a.user0.clone(),
                    grant: Some(authz::Grant {
                        authorization: Some(Any::pack(&authz::GenericAuthorization {
                            msg: "/cosmos.staking.v1beta1.MsgEditValidator".into(),
                        })),
                        expiration: None,
                    }),
                }),
                Any::pack(&authz::MsgGrant {
                    granter: a.val0.clone(),
                    grantee: a.user0.clone(),
                    grant: Some(authz::Grant {
                        authorization: Some(Any::pack(&authz::GenericAuthorization {
                            msg: "/cosmos.gov.v1.MsgVote".into(),
                        })),
                        expiration: None,
                    }),
                }),
            ],
            b"",
        ),
        c(
            "gov v1 proposal 2: MsgSetSendEnabled with an invalid denom (passes, execution fails: proposal_log)",
            "user0",
            false,
            vec![proposal(
                a,
                vec![Any::pack(&bank::MsgSetSendEnabled {
                    authority: GOV.into(),
                    send_enabled: vec![bank::SendEnabled {
                        denom: gs(b"bad\xff"),
                        enabled: true,
                    }],
                    use_default_for: Vec::new(),
                })],
                b"send enabled \xff",
                b"s",
                b"",
            )],
            b"",
        ),
        c(
            "gov v1 proposal 3: MsgSoftwareUpgrade with invalid plan name / info (passes; execution fails: circuit breaker)",
            "user0",
            false,
            vec![proposal(
                a,
                vec![Any::pack(
                    &celestia_proto::cosmos::upgrade::v1beta1::MsgSoftwareUpgrade {
                        authority: GOV.into(),
                        plan: Some(celestia_proto::cosmos::upgrade::v1beta1::Plan {
                            name: gs(b"v11-\xff"),
                            height: 1_000_000,
                            info: gs(b"\xfe info"),
                            ..Default::default()
                        }),
                    },
                )],
                b"upgrade \xfe",
                b"\x80",
                b"",
            )],
            b"",
        ),
        c(
            "authz exec: MsgEditValidator with invalid moniker, MsgVote with invalid metadata",
            "user0",
            false,
            vec![Any::pack(&authz::MsgExec {
                grantee: a.user0.clone(),
                msgs: vec![
                    edit_validator(
                        a,
                        staking::Description {
                            moniker: gs(b"exec-\xff"),
                            identity: "[do-not-modify]".into(),
                            website: "[do-not-modify]".into(),
                            security_contact: "[do-not-modify]".into(),
                            details: gs(b"\xfe"),
                        },
                    ),
                    Any::pack(&gov::MsgVote {
                        proposal_id: 2,
                        voter: a.val0.clone(),
                        option: gov::VoteOption::Yes as i32,
                        metadata: gs(b"exec vote \xff"),
                    }),
                ],
            })],
            b"",
        ),
        c(
            "val0 votes yes on proposal 3",
            "val0",
            false,
            vec![Any::pack(&gov::MsgVote {
                proposal_id: 3,
                voter: a.val0.clone(),
                option: gov::VoteOption::Yes as i32,
                metadata: String::new(),
            })],
            b"",
        ),
        c(
            "authz revoke of an invalid UTF-8 type URL (fails: not found)",
            "val0",
            false,
            vec![Any::pack(&authz::MsgRevoke {
                granter: a.val0.clone(),
                grantee: a.user0.clone(),
                msg_type_url: gs(b"/cosmos.\xff"),
            })],
            b"",
        ),
        c(
            "feegrant: allowed messages with invalid UTF-8",
            "val0",
            false,
            vec![Any::pack(&feegrant::MsgGrantAllowance {
                granter: a.val0.clone(),
                grantee: a.user0.clone(),
                allowance: Some(Any::pack(&feegrant::AllowedMsgAllowance {
                    allowance: Some(Any::pack(&feegrant::BasicAllowance {
                        spend_limit: utia(1_000_000),
                        expiration: None,
                    })),
                    allowed_messages: vec![
                        gs(b"/cosmos.bank.v1beta1.MsgSend\xff"),
                        "/cosmos.bank.v1beta1.MsgSend".into(),
                    ],
                })),
            })],
            b"",
        ),
        c(
            "amino-JSON signed send: memo with invalid UTF-8 (sign bytes carry \\ufffd)",
            "user0",
            true,
            vec![Any::pack(&bank::MsgSend {
                from_address: a.user0.clone(),
                to_address: a.val0.clone(),
                amount: utia(1),
            })],
            b"amino \xff \xe2\x82 <&> \xf4\x8f\xbf\xbf",
        ),
        c(
            "amino-JSON signed edit-validator with invalid moniker",
            "val0",
            true,
            vec![edit_validator(
                a,
                staking::Description {
                    moniker: gs(b"amino-\xfe"),
                    identity: "[do-not-modify]".into(),
                    website: "[do-not-modify]".into(),
                    security_contact: "[do-not-modify]".into(),
                    details: "[do-not-modify]".into(),
                },
            )],
            b"\x80",
        ),
    ]
}

/// Signs `case` for `key` (account number, sequence) on `chain_id`;
/// returns the `TxRaw` bytes.
pub fn sign(
    case: &Case,
    key: &keyring::KeyRecord,
    chain_id: &str,
    account_number: u64,
    sequence: u64,
) -> anyhow::Result<Vec<u8>> {
    let body = TxBody {
        messages: case.msgs.clone(),
        memo: case.memo.clone(),
        ..Default::default()
    };
    let mode = if case.amino {
        SignMode::LegacyAminoJson
    } else {
        SignMode::Direct
    };
    #[allow(deprecated)]
    let auth_info = AuthInfo {
        signer_infos: vec![SignerInfo {
            public_key: Some(key.pubkey_any()),
            mode_info: Some(ModeInfo {
                sum: Some(mode_info::Sum::Single(mode_info::Single {
                    mode: mode as i32,
                })),
            }),
            sequence,
        }],
        fee: Some(Fee {
            amount: utia(40_000),
            gas_limit: 800_000,
            payer: String::new(),
            granter: String::new(),
        }),
        tip: None,
    };
    let body_bytes = body.encode_to_vec();
    let auth_info_bytes = auth_info.encode_to_vec();
    let sign_bytes = if case.amino {
        let unsigned = TxRaw {
            body_bytes: body_bytes.clone(),
            auth_info_bytes: auth_info_bytes.clone(),
            signatures: vec![Vec::new()],
        }
        .encode_to_vec();
        let tx = txdecode::decode(&unsigned).map_err(|e| anyhow::anyhow!("{}", e.log))?;
        let sd = x_ante::signing::SignerData {
            address: key.bech32("celestia"),
            chain_id: chain_id.to_owned(),
            account_number,
            sequence,
            pub_key: key.pubkey_any(),
        };
        x_ante::aminojson::sign_bytes(&sd, &tx).map_err(|e| anyhow::anyhow!(e))?
    } else {
        SignDoc {
            body_bytes: body_bytes.clone(),
            auth_info_bytes: auth_info_bytes.clone(),
            chain_id: chain_id.to_owned(),
            account_number,
        }
        .encode_to_vec()
    };
    let sig = key
        .sign(&sign_bytes)
        .map_err(|e| anyhow::anyhow!("sign: {e}"))?;
    Ok(TxRaw {
        body_bytes,
        auth_info_bytes,
        signatures: vec![sig.to_vec()],
    }
    .encode_to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cases_carry_invalid_utf8() {
        let a = Accounts {
            val0: "celestia1a".into(),
            val0_oper: "celestiavaloper1a".into(),
            user0: "celestia1b".into(),
        };
        let cs = cases(&a);
        assert!(cs.len() > 10);
        let first = cs[0].msgs[0].encode_to_vec();
        assert!(first.windows(2).any(|w| w == b"\xff\xfe"));
        let m =
            staking::MsgEditValidator::decode(cs[1].msgs[0].value.as_slice()).unwrap_or_default();
        let moniker = m.description.unwrap_or_default().moniker;
        assert_eq!(gostr::go_len(&moniker), 70);
    }
}
