//! Go type names of the protobuf messages, as `codec/unknownproto` puts
//! them in its errors (`reflect.Type.String()` of a pointer to the
//! gogoproto-registered struct: `*<Go package>.<Name>`, nested messages
//! `Outer_Inner`). The Go package names come from each proto package's
//! `go_package` option in `proto/`; `google.protobuf` (`Any`, well-known
//! types) is gogoproto's / the SDK's `types`.

/// `(proto package, Go package name)`, sorted by proto package.
static GO_PACKAGES: &[(&str, &str)] = &[
    ("amino", "amino"),
    ("capability.v1", "types"),
    ("celestia.blob.v1", "types"),
    ("celestia.core.v1.da", "da"),
    ("celestia.core.v1.gas_estimation", "gasestimation"),
    ("celestia.core.v1.proof", "proof"),
    ("celestia.core.v1.tx", "tx"),
    ("celestia.fibre.v1", "types"),
    ("celestia.forwarding.v1", "types"),
    ("celestia.minfee.v1", "types"),
    ("celestia.mint.v1", "types"),
    ("celestia.signal.v1", "types"),
    ("celestia.valaddr.v1", "types"),
    ("celestia.zkism.v1", "types"),
    ("cosmos.auth.v1beta1", "types"),
    ("cosmos.authz.v1beta1", "authz"),
    ("cosmos.bank.v1beta1", "types"),
    ("cosmos.base.abci.v1beta1", "types"),
    ("cosmos.base.node.v1beta1", "node"),
    ("cosmos.base.query.v1beta1", "query"),
    ("cosmos.base.reflection.v1beta1", "reflection"),
    ("cosmos.base.reflection.v2alpha1", "v2alpha1"),
    ("cosmos.base.tendermint.v1beta1", "cmtservice"),
    ("cosmos.base.v1beta1", "types"),
    ("cosmos.circuit.v1", "types"),
    ("cosmos.consensus.v1", "types"),
    ("cosmos.crisis.v1beta1", "types"),
    ("cosmos.crypto.ed25519", "ed25519"),
    ("cosmos.crypto.hd.v1", "hd"),
    ("cosmos.crypto.keyring.v1", "keyring"),
    ("cosmos.crypto.multisig", "multisig"),
    ("cosmos.crypto.multisig.v1beta1", "types"),
    ("cosmos.crypto.secp256k1", "secp256k1"),
    ("cosmos.crypto.secp256r1", "secp256r1"),
    ("cosmos.distribution.v1beta1", "types"),
    ("cosmos.evidence.v1beta1", "types"),
    ("cosmos.feegrant.v1beta1", "feegrant"),
    ("cosmos.genutil.v1beta1", "types"),
    ("cosmos.gov.v1", "v1"),
    ("cosmos.gov.v1beta1", "v1beta1"),
    ("cosmos.ics23.v1", "ics23"),
    ("cosmos.mint.v1beta1", "types"),
    ("cosmos.msg.v1", "msgservice"),
    ("cosmos.params.v1beta1", "proposal"),
    ("cosmos.query.v1", "query"),
    ("cosmos.slashing.v1beta1", "types"),
    ("cosmos.staking.v1beta1", "types"),
    ("cosmos.store.internal.kv.v1beta1", "kv"),
    ("cosmos.store.snapshots.v1", "types"),
    ("cosmos.store.streaming.abci", "abci"),
    ("cosmos.store.v1beta1", "types"),
    ("cosmos.tx.signing.v1beta1", "signing"),
    ("cosmos.tx.v1beta1", "tx"),
    ("cosmos.upgrade.v1beta1", "types"),
    ("cosmos.vesting.v1beta1", "types"),
    ("cosmos_proto", "cosmos_proto"),
    ("gogoproto", "gogoproto"),
    ("google.api", "api"),
    ("google.protobuf", "types"),
    ("hyperlane.core.interchain_security.v1", "types"),
    ("hyperlane.core.post_dispatch.v1", "types"),
    ("hyperlane.core.v1", "types"),
    ("hyperlane.warp.v1", "types"),
    ("ibc.applications.fee.v1", "types"),
    (
        "ibc.applications.interchain_accounts.controller.v1",
        "types",
    ),
    ("ibc.applications.interchain_accounts.genesis.v1", "types"),
    ("ibc.applications.interchain_accounts.host.v1", "types"),
    ("ibc.applications.interchain_accounts.v1", "types"),
    ("ibc.applications.transfer.v1", "types"),
    ("ibc.applications.transfer.v2", "types"),
    ("ibc.core.channel.v1", "types"),
    ("ibc.core.client.v1", "types"),
    ("ibc.core.commitment.v1", "types"),
    ("ibc.core.connection.v1", "types"),
    ("ibc.core.types.v1", "types"),
    ("ibc.lightclients.localhost.v2", "localhost"),
    ("ibc.lightclients.solomachine.v2", "v7"),
    ("ibc.lightclients.solomachine.v3", "solomachine"),
    ("ibc.lightclients.tendermint.v1", "tendermint"),
    ("ibc.lightclients.wasm.v1", "types"),
    ("packetforward.v1", "types"),
    ("tendermint.abci", "types"),
    ("tendermint.blocksync", "blocksync"),
    ("tendermint.consensus", "consensus"),
    ("tendermint.crypto", "crypto"),
    ("tendermint.libs.bits", "bits"),
    ("tendermint.mempool", "mempool"),
    ("tendermint.p2p", "p2p"),
    ("tendermint.privval", "privval"),
    ("tendermint.propagation", "propagation"),
    ("tendermint.rpc.grpc", "coregrpc"),
    ("tendermint.state", "state"),
    ("tendermint.statesync", "statesync"),
    ("tendermint.store", "store"),
    ("tendermint.types", "types"),
    ("tendermint.version", "version"),
];

/// `reflect.TypeOf(msg).String()` of the Go message `full_name` (proto
/// full name without the leading dot); the full name when its package is
/// unknown.
#[must_use]
pub fn go_type_name(full_name: &str) -> String {
    let mut best: Option<(&str, &str)> = None;
    for &(pkg, go) in GO_PACKAGES {
        if full_name.len() > pkg.len()
            && full_name.starts_with(pkg)
            && full_name.as_bytes()[pkg.len()] == b'.'
            && best.is_none_or(|(p, _)| p.len() < pkg.len())
        {
            best = Some((pkg, go));
        }
    }
    match best {
        Some((pkg, go)) => format!("*{go}.{}", full_name[pkg.len() + 1..].replace('.', "_")),
        None => full_name.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::go_type_name;

    #[test]
    fn go_names() {
        // celestia-app v9.0.8 / Go: `errUnknownField "*types.MsgForward"`.
        assert_eq!(
            go_type_name("celestia.forwarding.v1.MsgForward"),
            "*types.MsgForward"
        );
        assert_eq!(go_type_name("cosmos.gov.v1.MsgVote"), "*v1.MsgVote");
        assert_eq!(go_type_name("cosmos.tx.v1beta1.TxBody"), "*tx.TxBody");
        assert_eq!(go_type_name("google.protobuf.Any"), "*types.Any");
        assert_eq!(
            go_type_name("cosmos.tx.v1beta1.ModeInfo.Single"),
            "*tx.ModeInfo_Single"
        );
    }
}
