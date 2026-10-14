//! Go JSON encodings of celestia-core values as the RPC server writes them:
//! `libs/json` (cmtjson: int64/uint64 as strings, `[]byte` as base64,
//! `HexBytes` as upper-case hex, registered interfaces as
//! `{"type","value"}`, `omitempty` honored) and gogoproto `jsonpb`
//! (`EmitDefaults`, `EnumsAsInts`) for the ABCI types that override
//! `MarshalJSON` (`ResponseCheckTx`, `ExecTxResult`, `ResponseQuery`,
//! `ResponseCommit`, `EventAttribute`).
//!
//! Empty byte slices follow protobuf decoding (nil), so they encode as
//! `null` where Go's field is a plain `[]byte`.

use ctypes::proto::abci;
use ctypes::proto::crypto as pcrypto;
use ctypes::proto::types as ptypes;
use ctypes::{
    Block, BlockId, BlockMeta, Commit, CommitSig, ConsensusParams, Data, Evidence, Header,
    LightBlock, PartSetHeader, PubKey, SignedHeader, Validator, ValidatorSet, Vote,
};

use crate::json::J;

/// `crypto.PubKey` (registered amino name + base64 key).
#[must_use]
pub fn pub_key(pk: &PubKey) -> J {
    let name = match pk {
        PubKey::Ed25519(_) => "tendermint/PubKeyEd25519",
        PubKey::Secp256k1(_) => "tendermint/PubKeySecp256k1",
    };
    J::obj([("type", J::str(name)), ("value", J::b64(pk.bytes()))])
}

/// `PartSetHeader`.
#[must_use]
pub fn part_set_header(p: &PartSetHeader) -> J {
    J::obj([("total", J::num(p.total)), ("hash", J::hex(&p.hash))])
}

/// `BlockID`.
#[must_use]
pub fn block_id(b: &BlockId) -> J {
    J::obj([
        ("hash", J::hex(&b.hash)),
        ("parts", part_set_header(&b.part_set_header)),
    ])
}

/// `Header`.
#[must_use]
pub fn header(h: &Header) -> J {
    let mut version = J::Obj(Vec::new());
    if h.version.block != 0 {
        version.push("block", J::u64s(h.version.block));
    }
    if h.version.app != 0 {
        version.push("app", J::u64s(h.version.app));
    }
    J::obj([
        ("version", version),
        ("chain_id", J::str(&h.chain_id)),
        ("height", J::i64s(h.height)),
        ("time", J::time(&h.time)),
        ("last_block_id", block_id(&h.last_block_id)),
        ("last_commit_hash", J::hex(&h.last_commit_hash)),
        ("data_hash", J::hex(&h.data_hash)),
        ("validators_hash", J::hex(&h.validators_hash)),
        ("next_validators_hash", J::hex(&h.next_validators_hash)),
        ("consensus_hash", J::hex(&h.consensus_hash)),
        ("app_hash", J::hex(&h.app_hash)),
        ("last_results_hash", J::hex(&h.last_results_hash)),
        ("evidence_hash", J::hex(&h.evidence_hash)),
        ("proposer_address", J::hex(&h.proposer_address)),
    ])
}

/// `CommitSig`.
#[must_use]
pub fn commit_sig(s: &CommitSig) -> J {
    J::obj([
        ("block_id_flag", J::num(s.block_id_flag)),
        ("validator_address", J::hex(&s.validator_address)),
        ("timestamp", J::time(&s.timestamp)),
        ("signature", J::b64n(&s.signature)),
    ])
}

/// `Commit`.
#[must_use]
pub fn commit(c: &Commit) -> J {
    J::obj([
        ("height", J::i64s(c.height)),
        ("round", J::num(c.round)),
        ("block_id", block_id(&c.block_id)),
        (
            "signatures",
            J::Arr(c.signatures.iter().map(commit_sig).collect()),
        ),
    ])
}

/// `Data` (as decoded from protobuf: `txs` is never nil).
#[must_use]
pub fn data(d: &Data) -> J {
    J::obj([
        ("txs", J::Arr(d.txs.iter().map(|t| J::b64(&t.0)).collect())),
        ("square_size", J::u64s(d.square_size)),
    ])
}

/// `Vote`.
#[must_use]
pub fn vote(v: &Vote) -> J {
    J::obj([
        ("type", J::num(v.vote_type)),
        ("height", J::i64s(v.height)),
        ("round", J::num(v.round)),
        ("block_id", block_id(&v.block_id)),
        ("timestamp", J::time(&v.timestamp)),
        ("validator_address", J::hex(&v.validator_address)),
        ("validator_index", J::num(v.validator_index)),
        ("signature", J::b64n(&v.signature)),
        ("extension", J::b64n(&v.extension)),
        ("extension_signature", J::b64n(&v.extension_signature)),
    ])
}

/// `Validator`.
#[must_use]
pub fn validator(v: &Validator) -> J {
    J::obj([
        ("address", J::hex(&v.address)),
        ("pub_key", pub_key(&v.pub_key)),
        ("voting_power", J::i64s(v.voting_power)),
        ("proposer_priority", J::i64s(v.proposer_priority)),
    ])
}

/// `ValidatorSet`.
#[must_use]
pub fn validator_set(vs: &ValidatorSet) -> J {
    J::obj([
        (
            "validators",
            J::Arr(vs.validators.iter().map(validator).collect()),
        ),
        ("proposer", vs.proposer().map_or(J::Null, validator)),
    ])
}

/// `SignedHeader`.
#[must_use]
pub fn signed_header(sh: &SignedHeader) -> J {
    J::obj([
        ("header", sh.header.as_ref().map_or(J::Null, header)),
        ("commit", sh.commit.as_ref().map_or(J::Null, commit)),
    ])
}

/// `LightBlock`.
#[must_use]
pub fn light_block(lb: &LightBlock) -> J {
    J::obj([
        (
            "signed_header",
            lb.signed_header.as_ref().map_or(J::Null, signed_header),
        ),
        (
            "validator_set",
            lb.validator_set.as_ref().map_or(J::Null, validator_set),
        ),
    ])
}

/// `Evidence` (registered interface).
#[must_use]
pub fn evidence(e: &Evidence) -> J {
    match e {
        Evidence::DuplicateVote(d) => J::obj([
            ("type", J::str("tendermint/DuplicateVoteEvidence")),
            (
                "value",
                J::obj([
                    ("vote_a", d.vote_a.as_ref().map_or(J::Null, vote)),
                    ("vote_b", d.vote_b.as_ref().map_or(J::Null, vote)),
                    ("TotalVotingPower", J::i64s(d.total_voting_power)),
                    ("ValidatorPower", J::i64s(d.validator_power)),
                    ("Timestamp", J::time(&d.timestamp)),
                ]),
            ),
        ]),
        Evidence::LightClientAttack(l) => J::obj([
            ("type", J::str("tendermint/LightClientAttackEvidence")),
            (
                "value",
                J::obj([
                    (
                        "ConflictingBlock",
                        l.conflicting_block.as_ref().map_or(J::Null, light_block),
                    ),
                    ("CommonHeight", J::i64s(l.common_height)),
                    (
                        "ByzantineValidators",
                        J::Arr(l.byzantine_validators.iter().map(validator).collect()),
                    ),
                    ("TotalVotingPower", J::i64s(l.total_voting_power)),
                    ("Timestamp", J::time(&l.timestamp)),
                ]),
            ),
        ]),
    }
}

/// `Block`.
#[must_use]
pub fn block(b: &Block) -> J {
    J::obj([
        ("header", header(&b.header)),
        ("data", data(&b.data)),
        (
            "evidence",
            J::obj([(
                "evidence",
                J::Arr(b.evidence.evidence.iter().map(evidence).collect()),
            )]),
        ),
        (
            "last_commit",
            b.last_commit.as_ref().map_or(J::Null, commit),
        ),
    ])
}

/// `BlockMeta`.
#[must_use]
pub fn block_meta(m: &BlockMeta) -> J {
    J::obj([
        ("block_id", block_id(&m.block_id)),
        ("block_size", J::i64s(m.block_size)),
        ("header", header(&m.header)),
        ("num_txs", J::i64s(m.num_txs)),
    ])
}

/// `types.ConsensusParams`.
#[must_use]
pub fn consensus_params(p: &ConsensusParams) -> J {
    J::obj([
        (
            "block",
            J::obj([
                ("max_bytes", J::i64s(p.block.max_bytes)),
                ("max_gas", J::i64s(p.block.max_gas)),
            ]),
        ),
        (
            "evidence",
            J::obj([
                ("max_age_num_blocks", J::i64s(p.evidence.max_age_num_blocks)),
                ("max_age_duration", J::i64s(p.evidence.max_age_duration)),
                ("max_bytes", J::i64s(p.evidence.max_bytes)),
            ]),
        ),
        (
            "validator",
            J::obj([(
                "pub_key_types",
                J::Arr(p.validator.pub_key_types.iter().map(J::str).collect()),
            )]),
        ),
        ("version", J::obj([("app", J::u64s(p.version.app))])),
        (
            "abci",
            J::obj([(
                "vote_extensions_enable_height",
                J::i64s(p.abci.vote_extensions_enable_height),
            )]),
        ),
    ])
}

fn duration_ns(d: Option<&pbjson_types::Duration>) -> i64 {
    d.map_or(0, |d| {
        d.seconds
            .saturating_mul(1_000_000_000)
            .saturating_add(i64::from(d.nanos))
    })
}

/// cmtproto `ConsensusParams` (pointers + `omitempty` fields).
#[must_use]
pub fn proto_consensus_params(p: &ptypes::ConsensusParams) -> J {
    let mut o = J::Obj(Vec::new());
    if let Some(b) = &p.block {
        let mut x = J::Obj(Vec::new());
        if b.max_bytes != 0 {
            x.push("max_bytes", J::i64s(b.max_bytes));
        }
        if b.max_gas != 0 {
            x.push("max_gas", J::i64s(b.max_gas));
        }
        o.push("block", x);
    }
    if let Some(e) = &p.evidence {
        let mut x = J::Obj(Vec::new());
        if e.max_age_num_blocks != 0 {
            x.push("max_age_num_blocks", J::i64s(e.max_age_num_blocks));
        }
        x.push(
            "max_age_duration",
            J::i64s(duration_ns(e.max_age_duration.as_ref())),
        );
        if e.max_bytes != 0 {
            x.push("max_bytes", J::i64s(e.max_bytes));
        }
        o.push("evidence", x);
    }
    if let Some(v) = &p.validator {
        let mut x = J::Obj(Vec::new());
        if !v.pub_key_types.is_empty() {
            x.push(
                "pub_key_types",
                J::Arr(v.pub_key_types.iter().map(J::str).collect()),
            );
        }
        o.push("validator", x);
    }
    if let Some(v) = &p.version {
        let mut x = J::Obj(Vec::new());
        if v.app != 0 {
            x.push("app", J::u64s(v.app));
        }
        o.push("version", x);
    }
    if let Some(a) = &p.abci {
        let mut x = J::Obj(Vec::new());
        if a.vote_extensions_enable_height != 0 {
            x.push(
                "vote_extensions_enable_height",
                J::i64s(a.vote_extensions_enable_height),
            );
        }
        o.push("abci", x);
    }
    o
}

// --- ABCI ------------------------------------------------------------------

/// `EventAttribute` (jsonpb).
#[must_use]
pub fn event_attribute(a: &abci::EventAttribute) -> J {
    J::obj([
        ("key", J::str(&a.key)),
        ("value", J::str(&a.value)),
        ("index", J::Bool(a.index)),
    ])
}

/// `abci.Event` (cmtjson; `omitempty` fields).
#[must_use]
pub fn event(e: &abci::Event) -> J {
    let mut o = J::Obj(Vec::new());
    if !e.r#type.is_empty() {
        o.push("type", J::str(&e.r#type));
    }
    if !e.attributes.is_empty() {
        o.push(
            "attributes",
            J::Arr(e.attributes.iter().map(event_attribute).collect()),
        );
    }
    o
}

/// jsonpb `repeated Event`.
fn pb_events(events: &[abci::Event]) -> J {
    J::Arr(
        events
            .iter()
            .map(|e| {
                J::obj([
                    ("type", J::str(&e.r#type)),
                    (
                        "attributes",
                        J::Arr(e.attributes.iter().map(event_attribute).collect()),
                    ),
                ])
            })
            .collect(),
    )
}

/// `[]abci.Event` (cmtjson: nil → `null`).
#[must_use]
pub fn events(evs: &[abci::Event]) -> J {
    if evs.is_empty() {
        J::Null
    } else {
        J::Arr(evs.iter().map(event).collect())
    }
}

/// `ExecTxResult` (jsonpb).
#[must_use]
pub fn exec_tx_result(r: &abci::ExecTxResult) -> J {
    J::obj([
        ("code", J::num(r.code)),
        ("data", J::b64n(&r.data)),
        ("log", J::str(&r.log)),
        ("info", J::str(&r.info)),
        ("gas_wanted", J::i64s(r.gas_wanted)),
        ("gas_used", J::i64s(r.gas_used)),
        ("events", pb_events(&r.events)),
        ("codespace", J::str(&r.codespace)),
        ("signers", J::Arr(r.signers.iter().map(J::str).collect())),
    ])
}

/// `ResponseCheckTx` (jsonpb).
#[must_use]
pub fn response_check_tx(r: &abci::ResponseCheckTx) -> J {
    J::obj([
        ("code", J::num(r.code)),
        ("data", J::b64n(&r.data)),
        ("log", J::str(&r.log)),
        ("info", J::str(&r.info)),
        ("gas_wanted", J::i64s(r.gas_wanted)),
        ("gas_used", J::i64s(r.gas_used)),
        ("events", pb_events(&r.events)),
        ("codespace", J::str(&r.codespace)),
        ("address", J::b64n(&r.address)),
        ("priority", J::i64s(r.priority)),
        ("sequence", J::u64s(r.sequence)),
    ])
}

fn proof_ops(p: &pcrypto::ProofOps) -> J {
    J::obj([(
        "ops",
        J::Arr(
            p.ops
                .iter()
                .map(|op| {
                    J::obj([
                        ("type", J::str(&op.r#type)),
                        ("key", J::b64n(&op.key)),
                        ("data", J::b64n(&op.data)),
                    ])
                })
                .collect(),
        ),
    )])
}

/// `ResponseQuery` (jsonpb) as returned in process: successful gRPC and
/// `app/*` / `custom/*` queries carry a non-nil (possibly empty) value,
/// store queries for a missing key and failures a nil one.
#[must_use]
pub fn response_query_for_path(path: &str, r: &abci::ResponseQuery) -> J {
    let mut j = response_query(r);
    if r.code == 0
        && r.value.is_empty()
        && !path.starts_with("/store")
        && !path.starts_with("store")
    {
        if let J::Obj(f) = &mut j {
            for (k, v) in f.iter_mut() {
                if k == "value" {
                    *v = J::str("");
                }
            }
        }
    }
    j
}

/// `ResponseQuery` (jsonpb).
#[must_use]
pub fn response_query(r: &abci::ResponseQuery) -> J {
    J::obj([
        ("code", J::num(r.code)),
        ("log", J::str(&r.log)),
        ("info", J::str(&r.info)),
        ("index", J::i64s(r.index)),
        ("key", J::b64n(&r.key)),
        ("value", J::b64n(&r.value)),
        ("proofOps", r.proof_ops.as_ref().map_or(J::Null, proof_ops)),
        ("height", J::i64s(r.height)),
        ("codespace", J::str(&r.codespace)),
    ])
}

/// `abci.TimeoutInfo` (cmtjson: durations as int64 nanoseconds).
#[must_use]
pub fn timeout_info(t: Option<&abci::TimeoutInfo>) -> J {
    let d = t.copied().unwrap_or_default();
    J::obj([
        (
            "timeout_propose",
            J::i64s(duration_ns(d.timeout_propose.as_ref())),
        ),
        (
            "timeout_propose_delta",
            J::i64s(duration_ns(d.timeout_propose_delta.as_ref())),
        ),
        (
            "timeout_prevote",
            J::i64s(duration_ns(d.timeout_prevote.as_ref())),
        ),
        (
            "timeout_prevote_delta",
            J::i64s(duration_ns(d.timeout_prevote_delta.as_ref())),
        ),
        (
            "timeout_precommit",
            J::i64s(duration_ns(d.timeout_precommit.as_ref())),
        ),
        (
            "timeout_precommit_delta",
            J::i64s(duration_ns(d.timeout_precommit_delta.as_ref())),
        ),
        (
            "timeout_commit",
            J::i64s(duration_ns(d.timeout_commit.as_ref())),
        ),
        (
            "delayed_precommit_timeout",
            J::i64s(duration_ns(d.delayed_precommit_timeout.as_ref())),
        ),
    ])
}

/// `ResponseInfo` (cmtjson; `omitempty` fields).
#[must_use]
pub fn response_info(r: &abci::ResponseInfo) -> J {
    let mut o = J::Obj(Vec::new());
    if !r.data.is_empty() {
        o.push("data", J::str(&r.data));
    }
    if !r.version.is_empty() {
        o.push("version", J::str(&r.version));
    }
    if r.app_version != 0 {
        o.push("app_version", J::u64s(r.app_version));
    }
    if r.last_block_height != 0 {
        o.push("last_block_height", J::i64s(r.last_block_height));
    }
    if !r.last_block_app_hash.is_empty() {
        o.push("last_block_app_hash", J::b64(&r.last_block_app_hash));
    }
    o.push("timeout_info", timeout_info(r.timeout_info.as_ref()));
    o
}

/// cmtproto `crypto.PublicKey` (cmtjson: `{"Sum": registered oneof}`).
#[must_use]
pub fn proto_public_key(pk: Option<&pcrypto::PublicKey>) -> J {
    use pcrypto::public_key::Sum;
    let sum = match pk.and_then(|p| p.sum.as_ref()) {
        None => J::Null,
        Some(Sum::Ed25519(b)) => J::obj([
            ("type", J::str("tendermint.crypto.PublicKey_Ed25519")),
            ("value", J::obj([("ed25519", J::b64n(b))])),
        ]),
        Some(Sum::Secp256k1(b)) => J::obj([
            ("type", J::str("tendermint.crypto.PublicKey_Secp256K1")),
            ("value", J::obj([("secp256k1", J::b64n(b))])),
        ]),
    };
    J::obj([("Sum", sum)])
}

/// `abci.ValidatorUpdate` (cmtjson).
#[must_use]
pub fn validator_update(v: &abci::ValidatorUpdate) -> J {
    let mut o = J::obj([("pub_key", proto_public_key(v.pub_key.as_ref()))]);
    if v.power != 0 {
        o.push("power", J::i64s(v.power));
    }
    o
}

/// `[]abci.ValidatorUpdate` (nil → `null`).
#[must_use]
pub fn validator_updates(v: &[abci::ValidatorUpdate]) -> J {
    if v.is_empty() {
        J::Null
    } else {
        J::Arr(v.iter().map(validator_update).collect())
    }
}

/// `abci.ResponseFinalizeBlock` (cmtjson).
#[must_use]
pub fn response_finalize_block(r: &abci::ResponseFinalizeBlock) -> J {
    let mut o = J::Obj(Vec::new());
    if !r.events.is_empty() {
        o.push("events", J::Arr(r.events.iter().map(event).collect()));
    }
    if !r.tx_results.is_empty() {
        o.push(
            "tx_results",
            J::Arr(r.tx_results.iter().map(exec_tx_result).collect()),
        );
    }
    o.push("validator_updates", validator_updates(&r.validator_updates));
    if let Some(p) = &r.consensus_param_updates {
        o.push("consensus_param_updates", proto_consensus_params(p));
    }
    if !r.app_hash.is_empty() {
        o.push("app_hash", J::b64(&r.app_hash));
    }
    o.push("timeout_info", timeout_info(r.timeout_info.as_ref()));
    o
}

/// `abci.TxResult` (cmtjson).
#[must_use]
pub fn tx_result(r: &abci::TxResult) -> J {
    let mut o = J::Obj(Vec::new());
    if r.height != 0 {
        o.push("height", J::i64s(r.height));
    }
    if r.index != 0 {
        o.push("index", J::num(r.index));
    }
    if !r.tx.is_empty() {
        o.push("tx", J::b64(&r.tx));
    }
    o.push(
        "result",
        r.result.as_ref().map_or_else(
            || exec_tx_result(&abci::ExecTxResult::default()),
            exec_tx_result,
        ),
    );
    o
}

// --- proofs ----------------------------------------------------------------

/// `merkle.Proof` (cmtjson).
#[must_use]
pub fn merkle_proof(total: i64, index: i64, leaf_hash: &[u8], aunts: &[Vec<u8>]) -> J {
    let mut o = J::obj([
        ("total", J::i64s(total)),
        ("index", J::i64s(index)),
        ("leaf_hash", J::b64n(leaf_hash)),
    ]);
    if !aunts.is_empty() {
        o.push("aunts", J::Arr(aunts.iter().map(|a| J::b64(a)).collect()));
    }
    o
}

/// cmtproto `NMTProof` (cmtjson, `omitempty` fields).
#[must_use]
pub fn nmt_proof(p: &ptypes::NmtProof) -> J {
    let mut o = J::Obj(Vec::new());
    if p.start != 0 {
        o.push("start", J::num(p.start));
    }
    if p.end != 0 {
        o.push("end", J::num(p.end));
    }
    if !p.nodes.is_empty() {
        o.push("nodes", J::Arr(p.nodes.iter().map(|n| J::b64(n)).collect()));
    }
    if !p.leaf_hash.is_empty() {
        o.push("leaf_hash", J::b64(&p.leaf_hash));
    }
    o
}

/// `types.ShareProof` built by `ShareProofFromProto`.
#[must_use]
pub fn share_proof(p: &ptypes::ShareProof) -> J {
    let rp = p.row_proof.clone();
    let row_proof = match rp {
        None => J::obj([
            ("row_roots", J::Null),
            ("proofs", J::Null),
            ("start_row", J::num(0)),
            ("end_row", J::num(0)),
        ]),
        Some(rp) => {
            // Go sizes `row_roots` by `len(RowRoots)` but fills it by
            // `len(Proofs)`.
            let roots: Vec<J> = (0..rp.row_roots.len())
                .map(|i| {
                    if i < rp.proofs.len() {
                        J::hex(&rp.row_roots[i])
                    } else {
                        J::hex(&[])
                    }
                })
                .collect();
            J::obj([
                ("row_roots", J::Arr(roots)),
                (
                    "proofs",
                    J::Arr(
                        rp.proofs
                            .iter()
                            .map(|pr| merkle_proof(pr.total, pr.index, &pr.leaf_hash, &pr.aunts))
                            .collect(),
                    ),
                ),
                ("start_row", J::num(rp.start_row)),
                ("end_row", J::num(rp.end_row)),
            ])
        }
    };
    J::obj([
        (
            "data",
            if p.data.is_empty() {
                J::Null
            } else {
                J::Arr(p.data.iter().map(|d| J::b64(d)).collect())
            },
        ),
        (
            "share_proofs",
            if p.share_proofs.is_empty() {
                J::Null
            } else {
                J::Arr(p.share_proofs.iter().map(nmt_proof).collect())
            },
        ),
        ("namespace_id", J::b64n(&p.namespace_id)),
        ("row_proof", row_proof),
        ("namespace_version", J::num(p.namespace_version)),
    ])
}

/// Whether a `ShareProof` is Go's zero value (omitted under `omitempty`).
#[must_use]
pub fn share_proof_is_zero(p: &ptypes::ShareProof) -> bool {
    p.data.is_empty()
        && p.share_proofs.is_empty()
        && p.namespace_id.is_empty()
        && p.row_proof.is_none()
        && p.namespace_version == 0
}
