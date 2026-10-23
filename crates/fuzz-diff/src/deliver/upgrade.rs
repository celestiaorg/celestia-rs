//! `DELIVER_UPGRADE=<n>`: the v9 → v10 upgrade inside `deliver`.
//!
//! The scenario (`v9-upgrade`, chain id `test`: signal upgrade delay 3)
//! is replayed up to the block before the recorded validator's first
//! `x/signal` tx; those txs are held back. The generated blocks then run
//! at app version 9 (Go v9.0.8) while the fuzz accounts build state that
//! crosses the boundary (validators, authz grants, fee allowances, vesting
//! accounts, gov proposals, hyperlane routers in v9's shapes, IBC packets
//! with `DELIVER_IBC`), signal and try to upgrade too early. From
//! generated block `n` the held validator signal is sent (the scenario's
//! validator holds almost all the power, so no other version can reach
//! the 5/6 quorum), the fuzz accounts send `MsgTryUpgrade` until the
//! upgrade is pending, then repeat it (`ErrUpgradePending`). At the
//! upgrade height `H` Go's v9 `EndBlocker` raises the app version; after
//! that `Commit` the harness switches Go to the v10.5.0 binary
//! (`DELIVER_SWITCH_CMD`, like the multiplexer / `abci-recorder`); the
//! Rust app stays the same process. Around `H` (and after) the driver
//! sends v10-only messages (fibre escrow, valaddr, warp
//! `MsgCreateNativeSyntheticToken`, `MsgForward` fields 7/8), uses the
//! grants, allowances and vesting accounts made at v9, transfers over the
//! v9 routers, votes the v9 proposals through (they end at v10), and
//! queries the signal / upgrade / fibre / valaddr state every block.

use super::*;

/// The scenario's `x/signal` txs (in block order) and the height of the
/// first block that holds one (`i64::MAX` when there is none).
pub(super) fn held_signals(records: &[abci_server::record::Record]) -> (i64, Vec<Vec<u8>>) {
    let mut first = i64::MAX;
    let mut txs = Vec::new();
    for r in records.iter().filter(|r| r.dir == Direction::Request) {
        let Ok(Request {
            value: Some(request::Value::FinalizeBlock(fb)),
        }) = Request::decode(r.bytes.as_slice())
        else {
            continue;
        };
        for t in &fb.txs {
            let signal = [9, 10].iter().any(|v| {
                type_urls(*v, t)
                    .iter()
                    .any(|u| u.starts_with("/celestia.signal.v1."))
            });
            if signal && !txs.contains(t) {
                first = first.min(fb.height);
                txs.push(t.clone());
            }
        }
    }
    (first, txs)
}

/// The height a replayed request belongs to, if it has one.
pub(super) fn request_height(req: &Request) -> Option<i64> {
    match req.value.as_ref()? {
        request::Value::PrepareProposal(p) => Some(p.height),
        request::Value::ProcessProposal(p) => Some(p.height),
        request::Value::FinalizeBlock(p) => Some(p.height),
        _ => None,
    }
}

/// Stops the Go app and starts the next binary on the same home and
/// address (`DELIVER_SWITCH_CMD`), then drops every connection: the
/// multiplexer's switch inside `Commit` (see `abci-recorder`).
pub(super) fn switch_go(go: &mut GoApp, from: u64, to: u64) -> anyhow::Result<()> {
    let cmd = std::env::var("DELIVER_SWITCH_CMD").map_err(|_| {
        anyhow::anyhow!(
            "Go raised the app version {from} -> {to}: set DELIVER_SWITCH_CMD \
             (deliver-diff.sh: GO_APPD_NEXT=<celestia-appd of the next version>)"
        )
    })?;
    let st = std::process::Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .status()?;
    anyhow::ensure!(st.success(), "switch command failed: {st}");
    go.conns.clear();
    Ok(())
}

/// A single-key account the driver creates on chain (vesting accounts).
fn new_acct(seed: &str) -> Acct {
    let mut secret = hash::sha256(seed.as_bytes());
    secret[0] &= 0x7f;
    let pub_key = crypto::secp256k1::public_key_from_secret_key(&secret).unwrap_or([2u8; 33]);
    let key = keyring::KeyRecord {
        name: seed.to_owned(),
        pub_key,
        secret: Some(secret),
        kind: keyring::KeyKind::Local,
    };
    Acct {
        addr: key.address().to_vec(),
        pk: key.pubkey_any(),
        acc: key.bech32("celestia"),
        val: key.bech32("celestiavaloper"),
        key,
        number: 0,
        seq: 0,
        ms: None,
    }
}

/// Domains of the routers enrolled at v9 with v9-only receiver shapes, and
/// the shapes.
const V9_ROUTERS: &[(u32, &str)] = &[
    (
        99,
        "0xABABABABABABABABABABABABABABABABABABABABABABABABABABABABABABABAB",
    ),
    (
        100,
        "abababababababababababababababababababababababababababababababab",
    ),
    (101, "0x1234"),
    (102, "not-a-hex-address"),
    (
        103,
        "0x00000000000000000000000000000000000000000000000000000000000ab0c0",
    ),
    (
        104,
        "0X00000000000000000000000000000000000000000000000000000000000ab0c0",
    ),
    (
        105,
        "0xzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    ),
    (
        106,
        "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\u{e9}",
    ),
];

/// State of the upgrade driver.
#[derive(Default)]
#[allow(clippy::struct_field_names)]
pub(super) struct Upgrade {
    pub on: bool,
    /// Generated block from which the held validator txs are sent.
    pub signal_at: usize,
    /// The scenario validator's recorded signal txs not yet finalized.
    pub held: Vec<Vec<u8>>,
    /// Upgrade height once the signal module holds a pending upgrade.
    pub upgrade_height: Option<i64>,
    /// First height executed at app version 10.
    pub v10_from: Option<i64>,
    /// Vesting accounts waiting for their creation tx: (account, tx).
    pub vesting_pending: Vec<(Acct, Vec<u8>)>,
    /// Vesting accounts created (indices into `gn.accts`).
    pub vesting: Vec<usize>,
    pub vest_n: usize,
    /// Proposals submitted at app version 9, voted yes by every account.
    pub voted: BTreeSet<u64>,
    /// The v9-shaped hyperlane routers were enrolled (step).
    pub routers: usize,
    /// Late at v10: the fuzz validators hold the power (see `txs`).
    pub late: bool,
    pub late_delegated: bool,
    /// The fuzz validator that takes the power.
    pub late_val: usize,
    /// The bonded validators' operators (late phase).
    pub bonded: Vec<String>,
    /// Notable results (signal / try-upgrade / boundary txs).
    pub log: Vec<String>,
    /// Proposal ids seen at app version 9.
    pub v9_proposals: BTreeSet<u64>,
    /// How the proposals of `v9_proposals` ended, by app version and result.
    pub v9_proposals_ended: BTreeMap<String, u64>,
    /// Txs the driver built, by type URL of their first message.
    pub built: BTreeMap<String, u64>,
}

impl Upgrade {
    /// The phase of `height` (volumes are counted per phase).
    pub fn phase(&self, height: i64, app_version: u64) -> String {
        if let Some(h) = self.upgrade_height {
            let d = height - h;
            return match d {
                ..=-4 => "v9 pending".into(),
                -3..=3 => format!("H{d:+}"),
                4..=20 => "v10 H+4..20".into(),
                _ => "v10 later".into(),
            };
        }
        if app_version >= 10 {
            "v10".into()
        } else if self.held.is_empty() {
            "v9 signalled".into()
        } else {
            "v9 before signal".into()
        }
    }

    /// The indices of the fuzz accounts that are validators.
    fn fuzz_validators(gn: &Gen) -> Vec<usize> {
        (0..ACCOUNTS)
            .filter(|&i| gn.validators.contains(&gn.accts[i].val))
            .collect()
    }

    fn push(&mut self, gn: &mut Gen, out: &mut Vec<Vec<u8>>, i: usize, msgs: Vec<Any>, gas: u64) {
        self.push_granted(gn, out, i, msgs, gas, None);
    }

    fn push_granted(
        &mut self,
        gn: &mut Gen,
        out: &mut Vec<Vec<u8>>,
        i: usize,
        msgs: Vec<Any>,
        gas: u64,
        granter: Option<String>,
    ) {
        let first = msgs.first().map(|m| m.type_url.clone()).unwrap_or_default();
        if let Some(t) = sign_with(gn, i, msgs, gas, granter.unwrap_or_default()) {
            gn.accts[i].seq += 1;
            *self.built.entry(first).or_default() += 1;
            out.push(t);
        }
    }

    /// The driver's txs of generated block `b` at `height`.
    #[allow(clippy::too_many_lines)]
    pub fn txs(&mut self, gn: &mut Gen, b: usize, height: i64, hl_token: &str) -> Vec<Vec<u8>> {
        use celestia_proto::celestia::signal::v1 as signal;
        let mut out = Vec::new();
        let v10 = gn.app_version >= 10;
        let near = self.upgrade_height.is_some_and(|h| (height - h).abs() <= 4);
        // Setup at v9: fuzz validators (a minority of the power).
        if !v10 && b < 4 {
            let i = b;
            let msg = Any::pack(
                &celestia_proto::cosmos::staking::v1beta1::MsgCreateValidator {
                    description: Some(celestia_proto::cosmos::staking::v1beta1::Description {
                        moniker: format!("upg{i}"),
                        ..Default::default()
                    }),
                    commission: Some(celestia_proto::cosmos::staking::v1beta1::CommissionRates {
                        rate: "100000000000000000".into(),
                        max_rate: "200000000000000000".into(),
                        max_change_rate: "10000000000000000".into(),
                    }),
                    min_self_delegation: "1".into(),
                    #[allow(deprecated)]
                    delegator_address: String::new(),
                    validator_address: gn.accts[i].val.clone(),
                    pubkey: Some(Any::pack(
                        &celestia_proto::cosmos::crypto::ed25519::PubKey {
                            key: hash::sha256(format!("upgrade validator {i}").as_bytes()).to_vec(),
                        },
                    )),
                    value: Some(Coin {
                        denom: "utia".into(),
                        amount: (1_000_000_000u64 * (1 + i as u64)).to_string(),
                    }),
                },
            );
            self.push(gn, &mut out, i, vec![msg], 400_000);
        }
        // An account at the fibre module address before the upgrade (v9
        // does not block it): a locked vesting account, then a send. The
        // v10 handler's `ensureFibreModuleAccount` replaces it.
        if !v10 && (2..4).contains(&b) {
            let fibre = module_address("fibre");
            let from = gn.accts[5].acc.clone();
            let utia = vec![Coin {
                denom: "utia".into(),
                amount: "777000".into(),
            }];
            let msg = if b == 2 {
                Any::pack(
                    &celestia_proto::cosmos::vesting::v1beta1::MsgCreatePermanentLockedAccount {
                        from_address: from,
                        to_address: fibre,
                        amount: utia,
                    },
                )
            } else {
                Any::pack(&celestia_proto::cosmos::bank::v1beta1::MsgSend {
                    from_address: from,
                    to_address: fibre,
                    amount: utia,
                })
            };
            self.push(gn, &mut out, 5, vec![msg], 300_000);
        }
        // Grants, allowances and vesting accounts made at v9, used at v10.
        if !v10 && (2..12).contains(&b) {
            self.v9_setup(gn, &mut out, b);
        }
        // Late at v10 the fuzz validators take over 5/6 of the power and
        // signal the current version (or 9). Go compares with
        // `BlockHeader().Version.App`, which `FinalizeBlock` never sets
        // (0): MsgTryUpgrade schedules an "upgrade" to 10 (or 9) that the
        // EndBlocker never applies, and the signal module stays pending
        // for good (security.md, open items). No signal above 10 from then
        // on (a v11 quorum would halt both apps).
        self.late = self.v10_from.is_some_and(|h| height >= h + 40);
        let vals = Self::fuzz_validators(gn);
        let bonded: Vec<usize> = vals
            .iter()
            .copied()
            .filter(|i| self.bonded.contains(&gn.accts[*i].val))
            .collect();
        if self.late && !self.late_delegated && !bonded.is_empty() {
            self.late_val = bonded[bonded.len() - 1];
            self.late_delegated = true;
            // The generator's own signals (any version) stop.
            let signal = "celestia.signal.v1.MsgSignalVersion";
            gn.types.retain(|md| md.full_name() != signal);
            gn.scenario_msgs.retain(|a| !a.type_url.ends_with(signal));
            // The bootstrap stake moves from the scenario's validator.
            let val0 = gn.validators.first().cloned().unwrap_or_default();
            for i in (0..ACCOUNTS - 1).filter(|&i| i != HL_ACCT) {
                use celestia_proto::cosmos::staking::v1beta1 as st;
                let coin = |n: &str| {
                    Some(Coin {
                        denom: "utia".into(),
                        amount: n.into(),
                    })
                };
                let msgs = vec![
                    Any::pack(&st::MsgUndelegate {
                        delegator_address: gn.accts[i].acc.clone(),
                        validator_address: val0.clone(),
                        amount: coin("100000000000000"),
                    }),
                    Any::pack(&st::MsgDelegate {
                        delegator_address: gn.accts[i].acc.clone(),
                        validator_address: gn.accts[self.late_val].val.clone(),
                        amount: coin("450000000000000"),
                    }),
                ];
                self.push(gn, &mut out, i, msgs, 1_000_000);
            }
        }
        // Validator signals: the fuzz validators (any version, mostly 10).
        if !vals.is_empty() && (gn.m.rng.one_in(3) || near || self.late) {
            let i = if self.late && vals.contains(&self.late_val) {
                self.late_val
            } else {
                vals[gn.m.rng.below(vals.len())]
            };
            let version = if self.late {
                [10, 10, 10, 9][gn.m.rng.below(4)]
            } else {
                [10, 10, 10, 9, 11, 0, u64::MAX, 12][gn.m.rng.below(8)]
            };
            let msg = Any::pack(&signal::MsgSignalVersion {
                validator_address: gn.accts[i].val.clone(),
                version,
            });
            self.push(gn, &mut out, i, vec![msg], 200_000);
        }
        // The scenario validator's held signal txs (in order).
        if b >= self.signal_at {
            out.extend(self.held.iter().cloned());
        }
        // MsgTryUpgrade: too early (no quorum), in the block of the
        // signal, until pending, repeated while pending, and at v10.
        let tries = if !self.held.is_empty() && b + 1 < self.signal_at {
            usize::from(gn.m.rng.one_in(4))
        } else if (self.upgrade_height.is_none() && !v10) || near || self.late {
            1 + gn.m.rng.below(2)
        } else {
            usize::from(gn.m.rng.one_in(5))
        };
        for _ in 0..tries {
            let i = gn.m.rng.below(ACCOUNTS);
            let msg = Any::pack(&signal::MsgTryUpgrade {
                signer: gn.accts[i].acc.clone(),
            });
            self.push(gn, &mut out, i, vec![msg], 200_000);
        }
        // Hyperlane routers in v9's shapes (stored as given at v9).
        if !v10 && !hl_token.is_empty() && self.routers == 0 {
            self.routers = 1;
            let msgs = V9_ROUTERS
                .iter()
                .map(|(d, r)| {
                    Any::pack(
                        &celestia_proto::hyperlane::warp::v1::MsgEnrollRemoteRouter {
                            owner: gn.accts[HL_ACCT].acc.clone(),
                            token_id: hl_token.to_owned(),
                            remote_router: Some(
                                celestia_proto::hyperlane::warp::v1::RemoteRouter {
                                    receiver_domain: *d,
                                    receiver_contract: (*r).to_owned(),
                                    gas: "50000".into(),
                                },
                            ),
                        },
                    )
                })
                .collect();
            self.push(gn, &mut out, HL_ACCT, msgs, 2_000_000);
        }
        // Proposals submitted while the upgrade is pending end at v10:
        // one that always passes (no message, or bank params as they are)
        // and one of a random gov message.
        if !v10 && self.upgrade_height.is_some() {
            let i = gn.m.rng.below(ACCOUNTS);
            #[allow(deprecated)]
            let msgs = if gn.m.rng.one_in(2) {
                Vec::new()
            } else {
                vec![Any::pack(
                    &celestia_proto::cosmos::bank::v1beta1::MsgUpdateParams {
                        authority: GOV.into(),
                        params: Some(celestia_proto::cosmos::bank::v1beta1::Params {
                            send_enabled: Vec::new(),
                            default_send_enabled: true,
                        }),
                    },
                )]
            };
            let p = Any::pack(&celestia_proto::cosmos::gov::v1::MsgSubmitProposal {
                messages: msgs,
                initial_deposit: vec![Coin {
                    denom: "utia".into(),
                    amount: "100000000000".into(),
                }],
                proposer: gn.accts[i].acc.clone(),
                metadata: String::new(),
                title: "pending".into(),
                summary: "pending".into(),
                expedited: false,
            });
            self.push(gn, &mut out, i, vec![p], 2_000_000);
            let i = (i + 1) % ACCOUNTS;
            let mut msgs = vec![gn.valid_gov_msg()];
            // Types v9 decodes (a proposal is stored as submitted and
            // unpacked again by v10's codec when it executes).
            let t: Vec<MessageDescriptor> = gn
                .types
                .iter()
                .filter(|md| registered(9, &format!("/{}", md.full_name())))
                .cloned()
                .collect();
            if !t.is_empty() && gn.m.rng.one_in(2) {
                let md = t[gn.m.rng.below(t.len())].clone();
                msgs.push(Any {
                    type_url: format!("/{}", md.full_name()),
                    value: gn.synth(&md, &Gen::gov_who(), 1),
                });
            }
            let p = Any::pack(&celestia_proto::cosmos::gov::v1::MsgSubmitProposal {
                messages: msgs,
                initial_deposit: vec![Coin {
                    denom: "utia".into(),
                    amount: "100000000000".into(),
                }],
                proposer: gn.accts[i].acc.clone(),
                metadata: String::new(),
                title: "pending".into(),
                summary: "pending".into(),
                expedited: gn.m.rng.one_in(2),
            });
            self.push(gn, &mut out, i, vec![p], 2_000_000);
        }
        // Proposals submitted at v9: every account votes yes.
        if !v10 || near {
            let ids: Vec<u64> = gn
                .proposals
                .iter()
                .copied()
                .filter(|p| !self.voted.contains(p))
                .collect();
            for id in ids.into_iter().take(2) {
                self.voted.insert(id);
                for i in 0..ACCOUNTS {
                    let vote = Any::pack(&celestia_proto::cosmos::gov::v1::MsgVote {
                        proposal_id: id,
                        voter: gn.accts[i].acc.clone(),
                        option: 1,
                        metadata: String::new(),
                    });
                    self.push(gn, &mut out, i, vec![vote], 300_000);
                }
            }
        }
        // The boundary: v10-only messages, v9 shapes, the v9 state used.
        let n = if near {
            6 + gn.m.rng.below(6)
        } else if self.upgrade_height.is_some() || gn.m.rng.one_in(3) {
            1 + gn.m.rng.below(3)
        } else {
            0
        };
        for _ in 0..n {
            self.boundary_tx(gn, &mut out, hl_token);
        }
        out
    }

    /// v9 setup of generated block `b`: authz grants (incl. for v10-only
    /// messages), fee allowances, vesting accounts, gov proposals.
    fn v9_setup(&mut self, gn: &mut Gen, out: &mut Vec<Vec<u8>>, b: usize) {
        use celestia_proto::cosmos::authz::v1beta1 as authz;
        use celestia_proto::cosmos::feegrant::v1beta1 as feegrant;
        use celestia_proto::cosmos::vesting::v1beta1 as vesting;
        let i = b % ACCOUNTS;
        let next = (i + 1) % ACCOUNTS;
        let utia = |n: u64| Coin {
            denom: "utia".into(),
            amount: n.to_string(),
        };
        // Grants of `i` to `i + 1` (the grantee `boundary_tx` executes as).
        let mut grants = Vec::new();
        for url in [
            "/cosmos.bank.v1beta1.MsgSend",
            "/celestia.fibre.v1.MsgDepositToEscrow",
            "/celestia.fibre.v1.MsgRequestWithdrawal",
            "/celestia.forwarding.v1.MsgForward",
            "/celestia.signal.v1.MsgTryUpgrade",
            "/hyperlane.warp.v1.MsgCreateNativeSyntheticToken",
        ] {
            grants.push(Any::pack(&authz::MsgGrant {
                granter: gn.accts[i].acc.clone(),
                grantee: gn.accts[next].acc.clone(),
                grant: Some(authz::Grant {
                    authorization: Some(Any::pack(&authz::GenericAuthorization {
                        msg: url.into(),
                    })),
                    expiration: None,
                }),
            }));
        }
        grants.push(Any::pack(&authz::MsgGrant {
            granter: gn.accts[i].acc.clone(),
            grantee: gn.accts[next].acc.clone(),
            grant: Some(authz::Grant {
                authorization: Some(Any::pack(
                    &celestia_proto::cosmos::bank::v1beta1::SendAuthorization {
                        spend_limit: vec![utia(50_000_000)],
                        allow_list: Vec::new(),
                    },
                )),
                expiration: Some(pbjson_types::Timestamp {
                    seconds: 1_900_000_000,
                    nanos: 0,
                }),
            }),
        }));
        self.push(gn, out, i, grants, 1_500_000);
        // Fee allowances of `i` to `i + 1`: plain, periodic and limited to
        // messages (some only v10 has).
        let basic = feegrant::BasicAllowance {
            spend_limit: vec![utia(5_000_000)],
            expiration: None,
        };
        let allowance = match b % 3 {
            0 => Any::pack(&basic),
            1 => Any::pack(&feegrant::PeriodicAllowance {
                basic: Some(basic),
                period: Some(pbjson_types::Duration {
                    seconds: 30,
                    nanos: 0,
                }),
                period_spend_limit: vec![utia(1_000_000)],
                period_can_spend: Vec::new(),
                period_reset: None,
            }),
            _ => Any::pack(&feegrant::AllowedMsgAllowance {
                allowance: Some(Any::pack(&basic)),
                allowed_messages: vec![
                    "/cosmos.bank.v1beta1.MsgSend".into(),
                    "/celestia.fibre.v1.MsgDepositToEscrow".into(),
                    "/celestia.valaddr.v1.MsgSetFibreProviderInfo".into(),
                ],
            }),
        };
        let g = Any::pack(&feegrant::MsgGrantAllowance {
            granter: gn.accts[i].acc.clone(),
            grantee: gn.accts[next].acc.clone(),
            allowance: Some(allowance),
        });
        self.push(gn, out, i, vec![g], 400_000);
        // A vesting account (continuous / delayed / periodic / locked)
        // ending around the upgrade.
        let a = new_acct(&format!("fuzz-diff upgrade vest {}", self.vest_n));
        self.vest_n += 1;
        let now = 1_700_000_000 + gn.height * 6;
        let msg = match b % 4 {
            0 | 1 => Any::pack(&vesting::MsgCreateVestingAccount {
                from_address: gn.accts[i].acc.clone(),
                to_address: a.acc.clone(),
                amount: vec![utia(400_000_000)],
                end_time: now + 600,
                delayed: b % 4 == 1,
                start_time: if b.is_multiple_of(8) { 0 } else { now - 60 },
            }),
            2 => Any::pack(&vesting::MsgCreatePeriodicVestingAccount {
                from_address: gn.accts[i].acc.clone(),
                to_address: a.acc.clone(),
                start_time: now - 60,
                vesting_periods: (0..4)
                    .map(|_| vesting::Period {
                        length: 120,
                        amount: vec![utia(100_000_000)],
                    })
                    .collect(),
            }),
            _ => Any::pack(&vesting::MsgCreatePermanentLockedAccount {
                from_address: gn.accts[i].acc.clone(),
                to_address: a.acc.clone(),
                amount: vec![utia(400_000_000)],
            }),
        };
        let first = msg.type_url.clone();
        if let Some(t) = sign_with(gn, i, vec![msg], 300_000, String::new()) {
            gn.accts[i].seq += 1;
            *self.built.entry(first).or_default() += 1;
            self.vesting_pending.push((a, t.clone()));
            out.push(t);
        }
        // Gov proposals submitted at v9 that end at v10: valid gov
        // messages and gov-signed messages of v9 types (re-packed by v10).
        let mut msgs = vec![gn.valid_gov_msg()];
        if gn.m.rng.one_in(2) {
            let t: Vec<MessageDescriptor> = gn.types.clone();
            if !t.is_empty() {
                let md = t[gn.m.rng.below(t.len())].clone();
                msgs.push(Any {
                    type_url: format!("/{}", md.full_name()),
                    value: gn.synth(&md, &Gen::gov_who(), 1),
                });
            }
        }
        let p = Any::pack(&celestia_proto::cosmos::gov::v1::MsgSubmitProposal {
            messages: msgs,
            initial_deposit: vec![utia(100_000_000_000)],
            proposer: gn.accts[next].acc.clone(),
            metadata: String::new(),
            title: "v9".into(),
            summary: "v9".into(),
            expedited: b.is_multiple_of(2),
        });
        self.push(gn, out, next, vec![p], 2_000_000);
    }

    /// One boundary tx: a v10-only message, a v9-shaped one, or one using
    /// state made at v9 (grants, allowances, vesting accounts, routers).
    #[allow(clippy::too_many_lines)]
    fn boundary_tx(&mut self, gn: &mut Gen, out: &mut Vec<Vec<u8>>, hl_token: &str) {
        use celestia_proto::celestia::fibre::v1 as fibre;
        use celestia_proto::cosmos::authz::v1beta1 as authz;
        let i = gn.m.rng.below(ACCOUNTS);
        let prev = (i + ACCOUNTS - 1) % ACCOUNTS;
        let me = gn.accts[i].acc.clone();
        let amount = |gn: &mut Gen| Coin {
            denom: if gn.m.rng.one_in(10) {
                "uatom".into()
            } else {
                "utia".into()
            },
            amount: [
                0u64,
                1,
                1000,
                1_000_000,
                1_000_000_000,
                10_000_000_000_000_000,
            ][gn.m.rng.below(6)]
            .to_string(),
        };
        let deposit = |gn: &mut Gen, signer: String| {
            Any::pack(&fibre::MsgDepositToEscrow {
                signer,
                amount: Some(amount(gn)),
            })
        };
        let forward = |gn: &mut Gen, signer: String, hooks: bool| {
            let id = gn.hex_id();
            Any::pack(&celestia_proto::celestia::forwarding::v1::MsgForward {
                signer,
                forward_addr: gn.accts[gn.m.rng.below(ACCOUNTS)].acc.clone(),
                dest_domain: [HL_REMOTE, 99, 1][gn.m.rng.below(3)],
                dest_recipient: format!("0x{}", "11".repeat(32)),
                token_id: if hl_token.is_empty() {
                    id.clone()
                } else {
                    hl_token.to_owned()
                },
                max_igp_fee: Some(Coin {
                    denom: "utia".into(),
                    amount: "1000000".into(),
                }),
                custom_hook_id: if hooks { id } else { String::new() },
                custom_hook_metadata: if hooks { "0x01".into() } else { String::new() },
            })
        };
        match gn.m.rng.below(14) {
            0 | 1 => {
                let m = deposit(gn, me);
                self.push(gn, out, i, vec![m], 300_000);
            }
            2 => {
                let m = Any::pack(&fibre::MsgRequestWithdrawal {
                    signer: me,
                    amount: Some(amount(gn)),
                });
                self.push(gn, out, i, vec![m], 300_000);
            }
            3 => {
                let host = ["fibre.example:7980", "", "127.0.0.1:1", "h"][gn.m.rng.below(4)];
                let m = Any::pack(
                    &celestia_proto::celestia::valaddr::v1::MsgSetFibreProviderInfo {
                        signer: me,
                        host: host.into(),
                    },
                );
                self.push(gn, out, i, vec![m], 300_000);
            }
            4 => {
                let hooks = gn.m.rng.one_in(2);
                let m = forward(gn, me, hooks);
                self.push(gn, out, i, vec![m], 1_000_000);
            }
            5 => {
                // MsgCreateNativeSyntheticToken: by an account (not the
                // authority) or in a proposal.
                let mailbox = gn
                    .hl
                    .mailbox_domain
                    .keys()
                    .next()
                    .cloned()
                    .unwrap_or_default();
                let m = Any::pack(
                    &celestia_proto::hyperlane::warp::v1::MsgCreateNativeSyntheticToken {
                        owner: GOV.into(),
                        origin_mailbox: mailbox,
                        origin_denom: format!("native{}", gn.m.rng.below(1000)),
                    },
                );
                let p = Any::pack(&celestia_proto::cosmos::gov::v1::MsgSubmitProposal {
                    messages: vec![m],
                    initial_deposit: vec![Coin {
                        denom: "utia".into(),
                        amount: "100000000000".into(),
                    }],
                    proposer: me,
                    metadata: String::new(),
                    title: "native".into(),
                    summary: "native".into(),
                    expedited: gn.m.rng.one_in(2),
                });
                self.push(gn, out, i, vec![p], 2_000_000);
            }
            6 | 7 => {
                // MsgExec by `i` of grants `i - 1` made at v9.
                let inner = match gn.m.rng.below(4) {
                    0 => Any::pack(&celestia_proto::cosmos::bank::v1beta1::MsgSend {
                        from_address: gn.accts[prev].acc.clone(),
                        to_address: me.clone(),
                        amount: vec![Coin {
                            denom: "utia".into(),
                            amount: "1000".into(),
                        }],
                    }),
                    1 => deposit(gn, gn.accts[prev].acc.clone()),
                    2 => Any::pack(&celestia_proto::celestia::signal::v1::MsgTryUpgrade {
                        signer: gn.accts[prev].acc.clone(),
                    }),
                    _ => {
                        let hooks = gn.m.rng.one_in(2);
                        forward(gn, gn.accts[prev].acc.clone(), hooks)
                    }
                };
                let m = Any::pack(&authz::MsgExec {
                    grantee: me,
                    msgs: vec![inner],
                });
                self.push(gn, out, i, vec![m], 1_000_000);
            }
            8 | 9 => {
                // Fees paid by `i - 1`'s allowance.
                let m = if gn.m.rng.one_in(2) {
                    deposit(gn, me.clone())
                } else {
                    Any::pack(&celestia_proto::cosmos::bank::v1beta1::MsgSend {
                        from_address: me.clone(),
                        to_address: gn.accts[prev].acc.clone(),
                        amount: vec![Coin {
                            denom: "utia".into(),
                            amount: "1".into(),
                        }],
                    })
                };
                let granter = Some(gn.accts[prev].acc.clone());
                self.push_granted(gn, out, i, vec![m], 300_000, granter);
            }
            10 | 11 if !self.vesting.is_empty() => {
                // A vesting account spends / delegates / escrows.
                let k = self.vesting[gn.m.rng.below(self.vesting.len())];
                let va = gn.accts[k].acc.clone();
                let val = gn.validators.first().cloned().unwrap_or_default();
                let m = match gn.m.rng.below(4) {
                    0 => Any::pack(&celestia_proto::cosmos::bank::v1beta1::MsgSend {
                        from_address: va,
                        to_address: me,
                        amount: vec![Coin {
                            denom: "utia".into(),
                            amount: ["1000", "150000000", "399000000"][gn.m.rng.below(3)].into(),
                        }],
                    }),
                    1 => Any::pack(&celestia_proto::cosmos::staking::v1beta1::MsgDelegate {
                        delegator_address: va,
                        validator_address: val,
                        amount: Some(Coin {
                            denom: "utia".into(),
                            amount: "200000000".into(),
                        }),
                    }),
                    2 => Any::pack(&celestia_proto::cosmos::staking::v1beta1::MsgUndelegate {
                        delegator_address: va,
                        validator_address: val,
                        amount: Some(Coin {
                            denom: "utia".into(),
                            amount: "100000000".into(),
                        }),
                    }),
                    _ => deposit(gn, va),
                };
                self.push(gn, out, k, vec![m], 400_000);
            }
            12 if !hl_token.is_empty() => {
                // A transfer over a router enrolled at v9.
                let (d, _) = V9_ROUTERS[gn.m.rng.below(V9_ROUTERS.len())];
                let m = Any::pack(&celestia_proto::hyperlane::warp::v1::MsgRemoteTransfer {
                    sender: gn.accts[HL_ACCT].acc.clone(),
                    token_id: hl_token.to_owned(),
                    destination_domain: d,
                    recipient: format!("0x{}", "22".repeat(32)),
                    amount: "1000".into(),
                    custom_hook_id: String::new(),
                    gas_limit: "0".into(),
                    max_fee: Some(Coin {
                        denom: "utia".into(),
                        amount: "10000000".into(),
                    }),
                    custom_hook_metadata: String::new(),
                });
                self.push(gn, out, HL_ACCT, vec![m], 1_000_000);
            }
            _ => {
                // The signal module from a non-validator / a fuzz validator.
                let vals = Self::fuzz_validators(gn);
                let v = gn.pick(&vals).unwrap_or(i);
                let m = Any::pack(&celestia_proto::celestia::signal::v1::MsgSignalVersion {
                    validator_address: gn.accts[v].val.clone(),
                    version: if self.late {
                        [9, 10][gn.m.rng.below(2)]
                    } else {
                        [9, 10, 11][gn.m.rng.below(3)]
                    },
                });
                self.push(gn, out, v, vec![m], 200_000);
            }
        }
    }

    /// After a finalized block: the held txs and vesting accounts it
    /// included, logged results of the driver's signal txs.
    #[allow(clippy::too_many_arguments)]
    pub fn observe(
        &mut self,
        gn: &mut Gen,
        rust: &mut app::App,
        txs: &[Vec<u8>],
        results: &[abci::ExecTxResult],
        block_events: &[abci::Event],
        block_version: u64,
        height: i64,
    ) {
        // Proposals submitted at app version 9 and how they ended.
        if block_version < 10 {
            for p in &gn.proposals {
                self.v9_proposals.insert(*p);
            }
        }
        for e in block_events
            .iter()
            .filter(|e| e.r#type == "active_proposal")
        {
            let get = |k: &str| {
                e.attributes
                    .iter()
                    .find(|a| a.key == k)
                    .map(|a| a.value.clone())
                    .unwrap_or_default()
            };
            if let Ok(id) = get("proposal_id").parse::<u64>()
                && self.v9_proposals.contains(&id)
            {
                self.log.push(format!(
                    "h={height} proposal {id} (submitted at v9) ended at app version {block_version}: {} {}",
                    get("proposal_result"),
                    get("proposal_log").chars().take(120).collect::<String>()
                ));
                *self
                    .v9_proposals_ended
                    .entry(format!("v{block_version} {}", get("proposal_result")))
                    .or_default() += 1;
            }
        }
        for (t, r) in txs.iter().zip(results) {
            let urls = type_urls(gn.app_version, t);
            if urls.iter().any(|u| u.contains("signal.v1.")) || self.held.contains(t) {
                // The signalled versions (and by which validator).
                let signals: Vec<String> = txdecode::decode_for(gn.app_version, t)
                    .map(|tx| {
                        tx.body
                            .messages
                            .iter()
                            .filter_map(|a| {
                                celestia_proto::celestia::signal::v1::MsgSignalVersion::decode(
                                    a.value.as_slice(),
                                )
                                .ok()
                                .filter(|_| a.type_url.ends_with("MsgSignalVersion"))
                                .map(|m| {
                                    let tail = m.validator_address.len().saturating_sub(6);
                                    format!("{}@{}", m.version, &m.validator_address[tail..])
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.log.push(format!(
                    "h={height} {}{urls:?}{signals:?} code {}/{}: {}",
                    if self.held.contains(t) { "held " } else { "" },
                    r.codespace,
                    r.code,
                    gostr::to_go_bytes(&r.log)
                        .escape_ascii()
                        .to_string()
                        .chars()
                        .take(140)
                        .collect::<String>()
                ));
            }
        }
        self.held.retain(|h| !txs.contains(h));
        let created: Vec<usize> = self
            .vesting_pending
            .iter()
            .enumerate()
            .filter(|(_, (_, t))| txs.iter().zip(results).any(|(x, r)| x == t && r.code == 0))
            .map(|(k, _)| k)
            .collect();
        for k in created.into_iter().rev() {
            let (mut a, _) = self.vesting_pending.remove(k);
            if let Some(n) = account_number(rust, &a.acc) {
                a.number = n;
                gn.accts.push(a);
                self.vesting.push(gn.accts.len() - 1);
            }
        }
        // Creation txs not in this block are dropped.
        self.vesting_pending.clear();
    }

    /// After `Commit`: the pending upgrade from the signal module.
    pub fn after_commit(&mut self, rust: &mut app::App, height: i64) {
        if self.v10_from.is_some_and(|h| height + 1 >= h + 40) && !self.late_delegated {
            self.bonded = bonded_validators(rust);
        }
        if self.late {
            let mut data = Vec::new();
            put_varint(&mut data, 10);
            let r = rust.query(&abci::RequestQuery {
                data: encode(&[Field {
                    num: 1,
                    wt: 0,
                    val: data,
                }]),
                path: "/celestia.signal.v1.Query/VersionTally".into(),
                height: 0,
                prove: false,
            });
            if let Ok(t) = celestia_proto::celestia::signal::v1::QueryVersionTallyResponse::decode(
                r.value.as_slice(),
            ) {
                self.log.push(format!(
                    "h={height} tally of 10: {} / threshold {} / total {}",
                    t.voting_power, t.threshold_power, t.total_voting_power
                ));
            }
        }
        if self.upgrade_height.is_some() {
            return;
        }
        let r = rust.query(&abci::RequestQuery {
            data: Vec::new(),
            path: "/celestia.signal.v1.Query/GetUpgrade".into(),
            height: 0,
            prove: false,
        });
        if let Ok(u) = celestia_proto::celestia::signal::v1::QueryGetUpgradeResponse::decode(
            r.value.as_slice(),
        ) && let Some(u) = u.upgrade
        {
            self.upgrade_height = Some(u.upgrade_height);
            self.log.push(format!(
                "h={height} upgrade pending: app version {} at height {}",
                u.app_version, u.upgrade_height
            ));
        }
    }
}

/// The account number of `acc` (Rust's committed state).
fn account_number(rust: &mut app::App, acc: &str) -> Option<u64> {
    use celestia_proto::cosmos::auth::v1beta1 as auth;
    let r = rust.query(&abci::RequestQuery {
        data: auth::QueryAccountInfoRequest {
            address: acc.to_owned(),
        }
        .encode_to_vec(),
        path: "/cosmos.auth.v1beta1.Query/AccountInfo".into(),
        height: 0,
        prove: false,
    });
    auth::QueryAccountInfoResponse::decode(r.value.as_slice())
        .ok()?
        .info
        .map(|i| i.account_number)
}

/// The queries of the upgrade state compared every block near the
/// boundary: signal tally / pending upgrade / missing validators,
/// `x/upgrade` plans and module versions, the fibre and valaddr state,
/// and the routers of the hyperlane token.
pub(super) fn queries(gn: &Gen, hl_token: &str) -> Vec<(&'static str, Vec<u8>)> {
    let n1 = |n: u64| {
        let mut v = Vec::new();
        put_varint(&mut v, n);
        encode(&[Field {
            num: 1,
            wt: 0,
            val: v,
        }])
    };
    let s1 = |s: &str| {
        encode(&[Field {
            num: 1,
            wt: 2,
            val: s.as_bytes().to_vec(),
        }])
    };
    let mut q: Vec<(&'static str, Vec<u8>)> = vec![
        ("/celestia.signal.v1.Query/GetUpgrade", Vec::new()),
        ("/cosmos.upgrade.v1beta1.Query/CurrentPlan", Vec::new()),
        ("/cosmos.upgrade.v1beta1.Query/AppliedPlan", s1("v10")),
        ("/cosmos.upgrade.v1beta1.Query/ModuleVersions", Vec::new()),
        ("/cosmos.upgrade.v1beta1.Query/ModuleVersions", s1("fibre")),
        ("/cosmos.consensus.v1.Query/Params", Vec::new()),
        ("/celestia.fibre.v1.Query/Params", Vec::new()),
        (
            "/celestia.valaddr.v1.Query/AllBondedFibreProviders",
            Vec::new(),
        ),
        ("/cosmos.auth.v1beta1.Query/ModuleAccounts", Vec::new()),
        (
            "/cosmos.auth.v1beta1.Query/ModuleAccountByName",
            s1("fibre"),
        ),
        ("/cosmos.evidence.v1beta1.Query/AllEvidence", Vec::new()),
        ("/cosmos.bank.v1beta1.Query/TotalSupply", Vec::new()),
    ];
    let fibre = module_address("fibre");
    for p in [
        "/cosmos.auth.v1beta1.Query/Account",
        "/cosmos.auth.v1beta1.Query/AccountInfo",
        "/cosmos.bank.v1beta1.Query/AllBalances",
        "/cosmos.bank.v1beta1.Query/SpendableBalances",
    ] {
        q.push((p, s1(&fibre)));
    }
    for v in [9, 10, 11, 0] {
        q.push(("/celestia.signal.v1.Query/VersionTally", n1(v)));
        q.push(("/celestia.signal.v1.Query/GetMissingValidators", n1(v)));
    }
    for a in gn
        .accts
        .iter()
        .take(4)
        .chain(gn.accts.iter().skip(ACCOUNTS + 4))
    {
        q.push(("/celestia.fibre.v1.Query/EscrowAccount", s1(&a.acc)));
        q.push(("/celestia.fibre.v1.Query/Withdrawals", s1(&a.acc)));
        q.push(("/cosmos.bank.v1beta1.Query/AllBalances", s1(&a.acc)));
        q.push(("/cosmos.bank.v1beta1.Query/SpendableBalances", s1(&a.acc)));
        q.push((
            "/cosmos.staking.v1beta1.Query/DelegatorDelegations",
            s1(&a.acc),
        ));
    }
    if !hl_token.is_empty() {
        q.push(("/hyperlane.warp.v1.Query/RemoteRouters", s1(hl_token)));
        q.push(("/hyperlane.warp.v1.Query/Token", s1(hl_token)));
        q.push(("/hyperlane.warp.v1.Query/BridgedSupply", s1(hl_token)));
        for (d, _) in V9_ROUTERS {
            let mut v = Vec::new();
            put_varint(&mut v, u64::from(*d));
            q.push((
                "/hyperlane.warp.v1.Query/QuoteRemoteTransfer",
                encode(&[
                    Field {
                        num: 1,
                        wt: 2,
                        val: hl_token.as_bytes().to_vec(),
                    },
                    Field {
                        num: 2,
                        wt: 2,
                        val: d.to_string().into_bytes(),
                    },
                ]),
            ));
        }
    }
    q
}
