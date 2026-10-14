//! `codectypes.UnpackInterfaces`: resolving `Any` values against the
//! interface they are declared as, as done by `ProtoCodec.Unmarshal` after
//! the gogoproto unmarshal.
//!
//! Each Go `UnpackInterfaces` method reachable from a tx is ported in
//! [`unpack_interfaces`], preserving call order: the `statefulUnpacker`
//! depth (10) and shared call budget (100) checks run before the nil /
//! empty-type-URL early returns, so the exact sequence of `UnpackAny` calls
//! decides whether a deeply nested tx fails.

use crate::gogo::{self, Dyn, GoError};
use crate::registry::{self, iface};

/// Why unpacking failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UnpackError {
    /// `UnpackInterfaces` returned an error.
    Err(GoError),
    /// Go panics (e.g. a value-receiver method called through a nil pointer).
    Panic(String),
}

impl From<GoError> for UnpackError {
    fn from(e: GoError) -> Self {
        Self::Err(e)
    }
}

type UResult<T> = Result<T, UnpackError>;

/// Go `MaxUnpackAnyRecursionDepth`.
const MAX_DEPTH: i32 = 10;
/// Go `MaxUnpackAnySubCalls`.
const MAX_CALLS: i32 = 100;

/// `statefulUnpacker`.
struct Unpacker<'a> {
    max_depth: i32,
    calls: &'a mut i32,
}

impl Unpacker<'_> {
    /// `statefulUnpacker.UnpackAny`. `any` is `None` for a nil `*Any`.
    fn unpack_any(&mut self, any: Option<&mut Dyn>, iface: u32) -> UResult<()> {
        if self.max_depth == 0 {
            return Err(GoError::new("max depth exceeded").into());
        }
        if *self.calls == 0 {
            return Err(GoError::new("call limit exceeded").into());
        }
        let Some(any) = any else { return Ok(()) };
        let type_url = any.bytes(1).to_vec();
        if type_url.is_empty() {
            return Ok(());
        }
        *self.calls -= 1;
        if !registry::has_impls(iface) {
            return Err(GoError::new("no registered implementations of type").into());
        }
        let Some(info) = registry::resolve_impl(&type_url, iface) else {
            return Err(GoError::new(format!(
                "no concrete type registered for type URL {} against interface",
                String::from_utf8_lossy(&type_url)
            ))
            .into());
        };
        let mut msg = Dyn::new(info);
        gogo::unmarshal_into(&mut msg, any.bytes(2))?;
        let mut child = Unpacker {
            max_depth: self.max_depth - 1,
            calls: &mut *self.calls,
        };
        unpack_interfaces(&mut msg, &mut child)?;
        any.cached = Some(Box::new(msg));
        Ok(())
    }
}

/// `registry.UnpackAny`: a fresh `statefulUnpacker` per call.
pub(crate) fn registry_unpack_any(any: Option<&mut Dyn>, iface: u32) -> UResult<()> {
    let mut calls = MAX_CALLS;
    let mut u = Unpacker {
        max_depth: MAX_DEPTH,
        calls: &mut calls,
    };
    u.unpack_any(any, iface)
}

/// `TxBody.UnpackInterfaces` with the registry as unpacker.
pub(crate) fn unpack_tx_body(body: &mut Dyn) -> UResult<()> {
    for any in body.msg_list_mut(1) {
        registry_unpack_any(Some(any), iface::MSG)?;
    }
    for num in [1023, 2047] {
        for any in body.msg_list_mut(num) {
            registry_unpack_any(Some(any), iface::TX_EXTENSION_OPTION)?;
        }
    }
    Ok(())
}

/// `AuthInfo.UnpackInterfaces` with the registry as unpacker.
pub(crate) fn unpack_auth_info(auth: &mut Dyn) -> UResult<()> {
    for signer_info in auth.msg_list_mut(1) {
        registry_unpack_any(signer_info.msg_mut(1), iface::PUB_KEY)?;
    }
    Ok(())
}

/// Messages (by full name) whose Go type implements
/// `UnpackInterfacesMessage` and is reachable through `UnpackAny` from a tx.
pub(crate) const IMPLEMENTED: &[&str] = &[
    "cosmos.authz.v1beta1.Grant",
    "cosmos.authz.v1beta1.MsgExec",
    "cosmos.authz.v1beta1.MsgGrant",
    "cosmos.crypto.multisig.LegacyAminoPubKey",
    "cosmos.evidence.v1beta1.MsgSubmitEvidence",
    "cosmos.feegrant.v1beta1.AllowedMsgAllowance",
    "cosmos.feegrant.v1beta1.MsgGrantAllowance",
    "cosmos.gov.v1.MsgExecLegacyContent",
    "cosmos.gov.v1.MsgSubmitProposal",
    "cosmos.gov.v1beta1.MsgSubmitProposal",
    "cosmos.staking.v1beta1.MsgCreateValidator",
    "ibc.core.client.v1.MsgCreateClient",
    "ibc.core.client.v1.MsgIBCSoftwareUpgrade",
    "ibc.core.client.v1.MsgSubmitMisbehaviour",
    "ibc.core.client.v1.MsgUpdateClient",
    "ibc.core.client.v1.MsgUpgradeClient",
    "ibc.core.client.v1.UpgradeProposal",
    "ibc.core.connection.v1.MsgConnectionOpenAck",
    "ibc.core.connection.v1.MsgConnectionOpenTry",
    "ibc.lightclients.solomachine.v3.ClientState",
    "ibc.lightclients.solomachine.v3.ConsensusState",
    "ibc.lightclients.solomachine.v3.Header",
];

/// `UnpackInterfaces(msg, unpacker)`.
fn unpack_interfaces(m: &mut Dyn, u: &mut Unpacker<'_>) -> UResult<()> {
    match m.info.name {
        // authz MsgExec: msgs as sdk.Msg.
        "cosmos.authz.v1beta1.MsgExec" => {
            for any in m.msg_list_mut(2) {
                u.unpack_any(Some(any), iface::MSG)?;
            }
            Ok(())
        }
        // authz MsgGrant: Grant (non-nullable) .UnpackInterfaces.
        "cosmos.authz.v1beta1.MsgGrant" => {
            let grant = m.msg_value_mut(3)?;
            unpack_interfaces(grant, u)
        }
        "cosmos.authz.v1beta1.Grant" => u.unpack_any(m.msg_mut(1), iface::AUTHORIZATION),
        "cosmos.crypto.multisig.LegacyAminoPubKey" => {
            for any in m.msg_list_mut(2) {
                u.unpack_any(Some(any), iface::PUB_KEY)?;
            }
            Ok(())
        }
        "cosmos.evidence.v1beta1.MsgSubmitEvidence" => u.unpack_any(m.msg_mut(2), iface::EVIDENCE),
        "cosmos.feegrant.v1beta1.AllowedMsgAllowance" => {
            u.unpack_any(m.msg_mut(1), iface::FEE_ALLOWANCE)
        }
        "cosmos.feegrant.v1beta1.MsgGrantAllowance" => {
            u.unpack_any(m.msg_mut(3), iface::FEE_ALLOWANCE)
        }
        "cosmos.gov.v1.MsgExecLegacyContent" | "cosmos.gov.v1beta1.MsgSubmitProposal" => {
            u.unpack_any(m.msg_mut(1), iface::CONTENT)
        }
        "cosmos.gov.v1.MsgSubmitProposal" => {
            for any in m.msg_list_mut(1) {
                u.unpack_any(Some(any), iface::MSG)?;
            }
            Ok(())
        }
        "cosmos.staking.v1beta1.MsgCreateValidator" => u.unpack_any(m.msg_mut(6), iface::PUB_KEY),
        "ibc.core.client.v1.MsgCreateClient" => {
            u.unpack_any(m.msg_mut(1), iface::CLIENT_STATE)?;
            u.unpack_any(m.msg_mut(2), iface::CONSENSUS_STATE)
        }
        "ibc.core.client.v1.MsgUpdateClient" | "ibc.core.client.v1.MsgSubmitMisbehaviour" => {
            u.unpack_any(m.msg_mut(2), iface::CLIENT_MESSAGE)
        }
        "ibc.core.client.v1.MsgUpgradeClient" => {
            u.unpack_any(m.msg_mut(2), iface::CLIENT_STATE)?;
            u.unpack_any(m.msg_mut(3), iface::CONSENSUS_STATE)
        }
        "ibc.core.client.v1.MsgIBCSoftwareUpgrade" => {
            u.unpack_any(m.msg_mut(2), iface::CLIENT_STATE)
        }
        "ibc.core.client.v1.UpgradeProposal" => u.unpack_any(m.msg_mut(4), iface::CLIENT_STATE),
        "ibc.core.connection.v1.MsgConnectionOpenTry" => {
            u.unpack_any(m.msg_mut(3), iface::CLIENT_STATE)
        }
        "ibc.core.connection.v1.MsgConnectionOpenAck" => {
            u.unpack_any(m.msg_mut(4), iface::CLIENT_STATE)
        }
        // solomachine ClientState: `cs.ConsensusState.UnpackInterfaces` with a
        // value receiver through a nullable pointer: nil panics.
        "ibc.lightclients.solomachine.v3.ClientState" => match m.msg_mut(3) {
            Some(cs) => unpack_interfaces(cs, u),
            None => Err(UnpackError::Panic(
                "runtime error: invalid memory address or nil pointer dereference".into(),
            )),
        },
        "ibc.lightclients.solomachine.v3.ConsensusState" => {
            u.unpack_any(m.msg_mut(1), iface::PUB_KEY)
        }
        "ibc.lightclients.solomachine.v3.Header" => u.unpack_any(m.msg_mut(3), iface::PUB_KEY),
        _ => Ok(()),
    }
}
