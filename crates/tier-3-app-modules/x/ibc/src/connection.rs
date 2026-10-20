//! ICS-03 connection submodule (`core/03-connection`): storage, params,
//! the sentinel localhost connection and genesis.

use celestia_proto::ibc::core::client::v1::Height;
use celestia_proto::ibc::core::commitment::v1::MerklePrefix;
use celestia_proto::ibc::core::connection::v1::{
    ClientPaths, ConnectionEnd, Counterparty, GenesisState, Params, State, Version,
};
use prost::Message;
use sdk::{Context, Result, SdkError};

use crate::client;
use crate::errors::{client as clerr, connection as cerr};
use crate::host;

/// Sub-module name.
pub const SUB_MODULE_NAME: &str = "connection";
/// `KeyNextConnectionSequence`.
pub const KEY_NEXT_CONNECTION_SEQUENCE: &[u8] = b"nextConnectionSequence";
/// `ParamsKey`.
pub const PARAMS_KEY: &[u8] = b"connectionParams";
/// `ConnectionPrefix`.
pub const CONNECTION_PREFIX: &str = "connection-";
/// `exported.LocalhostConnectionID`.
pub const LOCALHOST_CONNECTION_ID: &str = "connection-localhost";
/// `DefaultIBCVersionIdentifier`.
pub const DEFAULT_IBC_VERSION_IDENTIFIER: &str = "1";
/// `SupportedOrderings`.
pub const SUPPORTED_ORDERINGS: &[&str] = &["ORDER_ORDERED", "ORDER_UNORDERED"];

/// `GetCompatibleVersions`: `[DefaultIBCVersion]`.
#[must_use]
pub fn get_compatible_versions() -> Vec<Version> {
    vec![Version {
        identifier: DEFAULT_IBC_VERSION_IDENTIFIER.to_owned(),
        features: SUPPORTED_ORDERINGS
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
    }]
}

/// Fills gogoproto non-nullable embedded messages (`counterparty`,
/// `counterparty.prefix`) so they are always encoded, as Go does.
#[must_use]
pub fn normalize_connection(mut c: ConnectionEnd) -> ConnectionEnd {
    let mut cp = c.counterparty.take().unwrap_or_default();
    if cp.prefix.is_none() {
        cp.prefix = Some(MerklePrefix::default());
    }
    c.counterparty = Some(cp);
    c
}

/// `Params.Validate`.
pub fn validate_params(p: &Params) -> Result<()> {
    if p.max_expected_time_per_block == 0 {
        return Err(crate::errors::unregistered(
            "MaxExpectedTimePerBlock cannot be zero",
        ));
    }
    Ok(())
}

/// `SetConnection`.
pub fn set_connection(ctx: &mut Context<'_>, connection_id: &str, c: &ConnectionEnd) -> Result<()> {
    ctx.set(
        crate::STORE_KEY,
        host::connection_key(connection_id),
        normalize_connection(c.clone()).encode_to_vec(),
    )
}

/// `GetConnection`.
pub fn get_connection(ctx: &mut Context<'_>, connection_id: &str) -> Result<Option<ConnectionEnd>> {
    ctx.get(crate::STORE_KEY, &host::connection_key(connection_id))?
        .map(|b| {
            ConnectionEnd::decode(b.as_slice())
                .map_err(|e| sdk::error::sdk::json_unmarshal(e.to_string()))
        })
        .transpose()
}

/// `SetClientConnectionPaths`.
pub fn set_client_connection_paths(
    ctx: &mut Context<'_>,
    client_id: &str,
    paths: &[String],
) -> Result<()> {
    let cp = ClientPaths {
        paths: paths.to_vec(),
    };
    ctx.set(
        crate::STORE_KEY,
        host::client_connections_key(client_id),
        cp.encode_to_vec(),
    )
}

/// `SetNextConnectionSequence`.
pub fn set_next_connection_sequence(ctx: &mut Context<'_>, seq: u64) -> Result<()> {
    ctx.set(
        crate::STORE_KEY,
        KEY_NEXT_CONNECTION_SEQUENCE.to_vec(),
        seq.to_be_bytes().to_vec(),
    )
}

/// `GetNextConnectionSequence`.
pub fn get_next_connection_sequence(ctx: &mut Context<'_>) -> Result<u64> {
    let bz = ctx
        .get(crate::STORE_KEY, KEY_NEXT_CONNECTION_SEQUENCE)?
        .unwrap_or_default();
    crate::be_u64(&bz).ok_or_else(|| sdk::error::sdk::logic("next connection sequence is nil"))
}

/// `SetParams`.
pub fn set_params(ctx: &mut Context<'_>, p: &Params) -> Result<()> {
    ctx.set(crate::STORE_KEY, PARAMS_KEY.to_vec(), p.encode_to_vec())
}

/// `GetParams`.
pub fn get_params(ctx: &mut Context<'_>) -> Result<Params> {
    let bz = ctx
        .get(crate::STORE_KEY, PARAMS_KEY)?
        .ok_or_else(|| sdk::error::sdk::logic("ibc connection params are not set in store"))?;
    Params::decode(bz.as_slice()).map_err(|e| sdk::error::sdk::json_unmarshal(e.to_string()))
}

/// `CreateSentinelLocalhostConnection`: an OPEN connection from
/// 09-localhost to itself with the `ibc` commitment prefix.
pub fn create_sentinel_localhost_connection(ctx: &mut Context<'_>) -> Result<()> {
    let end = ConnectionEnd {
        client_id: crate::client::LOCALHOST_CLIENT_ID.to_owned(),
        versions: get_compatible_versions(),
        state: State::Open as i32,
        counterparty: Some(Counterparty {
            client_id: crate::client::LOCALHOST_CLIENT_ID.to_owned(),
            connection_id: LOCALHOST_CONNECTION_ID.to_owned(),
            prefix: Some(MerklePrefix {
                key_prefix: crate::COMMITMENT_PREFIX.to_vec(),
            }),
        }),
        delay_period: 0,
    };
    set_connection(ctx, LOCALHOST_CONNECTION_ID, &end)
}

/// Connection `InitGenesis`.
pub fn init_genesis(ctx: &mut Context<'_>, gs: &GenesisState) -> Result<()> {
    for c in &gs.connections {
        let end = ConnectionEnd {
            client_id: c.client_id.clone(),
            versions: c.versions.clone(),
            state: c.state,
            counterparty: c.counterparty.clone(),
            delay_period: c.delay_period,
        };
        set_connection(ctx, &c.id, &end)?;
    }
    for p in &gs.client_connection_paths {
        set_client_connection_paths(ctx, &p.client_id, &p.paths)?;
    }
    set_next_connection_sequence(ctx, gs.next_connection_sequence)?;
    set_params(ctx, &gs.params.unwrap_or_default())?;
    create_sentinel_localhost_connection(ctx)
}

// ---------------------------------------------------------------------------
// Versions (`types/version.go`)
// ---------------------------------------------------------------------------

/// `ValidateVersion`.
pub fn validate_version(v: Option<&Version>) -> Result<()> {
    let Some(v) = v else {
        return Err(cerr::invalid_version("version cannot be nil"));
    };
    if v.identifier.trim().is_empty() {
        return Err(cerr::invalid_version("version identifier cannot be blank"));
    }
    for (i, f) in v.features.iter().enumerate() {
        if f.trim().is_empty() {
            return Err(cerr::invalid_version(format!(
                "feature cannot be blank, index {i}"
            )));
        }
    }
    Ok(())
}

fn allow_nil_feature_set(identifier: &str) -> bool {
    // `allowNilFeatureSet` only lists "1" (= false).
    let _ = identifier;
    false
}

/// `Version.VerifyProposedVersion`.
fn verify_proposed_version(supported: &Version, proposed: &Version) -> Result<()> {
    if proposed.identifier != supported.identifier {
        return Err(cerr::version_negotiation_failed(
            "proposed version identifier does not equal supported version identifier",
        ));
    }
    if proposed.features.is_empty() && !allow_nil_feature_set(&proposed.identifier) {
        return Err(cerr::version_negotiation_failed(
            "nil feature sets are not supported",
        ));
    }
    for f in &proposed.features {
        if !supported.features.contains(f) {
            return Err(cerr::version_negotiation_failed(format!(
                "proposed feature ({f}) is not a supported feature set"
            )));
        }
    }
    Ok(())
}

/// `IsSupportedVersion`.
#[must_use]
pub fn is_supported_version(supported: &[Version], proposed: &Version) -> bool {
    supported
        .iter()
        .find(|s| s.identifier == proposed.identifier)
        .is_some_and(|s| verify_proposed_version(s, proposed).is_ok())
}

/// `VerifySupportedFeature`.
#[must_use]
pub fn verify_supported_feature(v: &Version, feature: &str) -> bool {
    v.features.iter().any(|f| f == feature)
}

/// `PickVersion`.
pub fn pick_version(supported: &[Version], counterparty: &[Version]) -> Result<Version> {
    for s in supported {
        if let Some(c) = counterparty.iter().find(|c| c.identifier == s.identifier) {
            let features: Vec<String> = s
                .features
                .iter()
                .filter(|f| c.features.contains(f))
                .cloned()
                .collect();
            if features.is_empty() && !allow_nil_feature_set(&s.identifier) {
                continue;
            }
            return Ok(Version {
                identifier: s.identifier.clone(),
                features,
            });
        }
    }
    Err(cerr::version_negotiation_failed(format!(
        "failed to find a matching counterparty version ({}) from the supported version list ({})",
        go_versions_string(counterparty),
        go_versions_string(supported)
    )))
}

/// gogoproto `CompactTextString` of a `Version` (`%s` / `%v` of a
/// `*Version`): `identifier:"1" features:"ORDER_ORDERED" `.
fn go_version_string(v: &Version) -> String {
    use std::fmt::Write as _;
    let quote = x_ante::gogotext::quote;
    let mut s = String::new();
    if !v.identifier.is_empty() {
        let _ = write!(s, "identifier:{} ", quote(&v.identifier));
    }
    for f in &v.features {
        let _ = write!(s, "features:{} ", quote(f));
    }
    s
}

/// `%v` of a `[]*Version`: the elements' text forms, space-separated in `[]`.
fn go_versions_string(vs: &[Version]) -> String {
    let items: Vec<String> = vs.iter().map(go_version_string).collect();
    format!("[{}]", items.join(" "))
}

// ---------------------------------------------------------------------------
// Identifiers and validation
// ---------------------------------------------------------------------------

/// `IsValidConnectionID`: `^connection-[0-9]{1,20}$` with a `uint64` sequence.
#[must_use]
pub fn is_valid_connection_id(id: &str) -> bool {
    id.strip_prefix(CONNECTION_PREFIX)
        .is_some_and(|n| !n.is_empty() && n.len() <= 20 && crate::height::parse_u64(n).is_some())
}

/// `Counterparty.ValidateBasic`.
pub fn validate_counterparty(c: &Counterparty) -> Result<()> {
    if !c.connection_id.is_empty() {
        host::connection_identifier_validator(&c.connection_id)
            .map_err(|e| e.wrap("invalid counterparty connection ID"))?;
    }
    host::client_identifier_validator(&c.client_id)
        .map_err(|e| e.wrap("invalid counterparty client ID"))?;
    if c.prefix.as_ref().is_none_or(|p| p.key_prefix.is_empty()) {
        return Err(cerr::invalid_counterparty(
            "counterparty prefix cannot be empty",
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Keeper
// ---------------------------------------------------------------------------

/// `GenerateConnectionIdentifier`.
pub fn generate_connection_identifier(ctx: &mut Context<'_>) -> Result<String> {
    let seq = get_next_connection_sequence(ctx)?;
    set_next_connection_sequence(ctx, seq.wrapping_add(1))?;
    Ok(format!("{CONNECTION_PREFIX}{seq}"))
}

/// `GetClientConnectionPaths`.
pub fn get_client_connection_paths(
    ctx: &mut Context<'_>,
    client_id: &str,
) -> Result<Option<Vec<String>>> {
    let bz = ctx
        .get(crate::STORE_KEY, &host::client_connections_key(client_id))?
        .unwrap_or_default();
    if bz.is_empty() {
        return Ok(None);
    }
    ClientPaths::decode(bz.as_slice())
        .map(|p| Some(p.paths))
        .map_err(crate::errors::must_unmarshal)
}

/// `addConnectionToClient`.
fn add_connection_to_client(ctx: &mut Context<'_>, client_id: &str, conn_id: &str) -> Result<()> {
    if client::get_client_state(ctx, client_id)?.is_none() {
        return Err(clerr::client_not_found(client_id));
    }
    let mut conns = get_client_connection_paths(ctx, client_id)?.unwrap_or_default();
    conns.push(conn_id.to_owned());
    set_client_connection_paths(ctx, client_id, &conns)
}

/// The commitment prefix of this chain (`ibc`).
#[must_use]
pub fn commitment_prefix() -> MerklePrefix {
    MerklePrefix {
        key_prefix: crate::COMMITMENT_PREFIX.to_vec(),
    }
}

/// `GetTimestampAtHeight` of a connection's client.
pub fn get_timestamp_at_height(
    ctx: &mut Context<'_>,
    conn: &ConnectionEnd,
    h: &Height,
) -> Result<u64> {
    let cs = client::client_state(ctx, &conn.client_id)?
        .ok_or_else(|| clerr::client_not_found(format!("clientID ({})", conn.client_id)))?;
    client::get_timestamp_at_height(ctx, &conn.client_id, &cs, h)
}

fn conn_events(
    ty: &str,
    attrs: Vec<(&str, String)>,
) -> Vec<celestia_proto::celestia_core_proto::abci::Event> {
    vec![
        sdk::event::event(ty, attrs),
        sdk::event::event("message", [("module", "ibc_connection")]),
    ]
}

fn state_name(s: i32) -> std::borrow::Cow<'static, str> {
    // gogoproto `EnumName`: an unknown value prints as its number.
    std::borrow::Cow::Borrowed(match State::try_from(s) {
        Ok(State::UninitializedUnspecified) => "STATE_UNINITIALIZED_UNSPECIFIED",
        Ok(State::Init) => "STATE_INIT",
        Ok(State::Tryopen) => "STATE_TRYOPEN",
        Ok(State::Open) => "STATE_OPEN",
        Err(_) => return std::borrow::Cow::Owned(s.to_string()),
    })
}

/// `ConnOpenInit`.
pub fn conn_open_init(
    ctx: &mut Context<'_>,
    client_id: &str,
    counterparty: &Counterparty,
    version: Option<&Version>,
    delay_period: u64,
) -> Result<String> {
    let mut versions = get_compatible_versions();
    if let Some(v) = version {
        if !is_supported_version(&get_compatible_versions(), v) {
            return Err(cerr::invalid_version("version is not supported"));
        }
        versions = vec![v.clone()];
    }
    let cs = client::client_state(ctx, client_id)?
        .ok_or_else(|| clerr::client_not_found(format!("clientID ({client_id})")))?;
    let status = client::client_status(ctx, client_id, &cs)?;
    if status != client::Status::Active {
        return Err(clerr::client_not_active(format!(
            "client ({client_id}) status is {}",
            status.as_str()
        )));
    }
    let conn_id = generate_connection_identifier(ctx)?;
    add_connection_to_client(ctx, client_id, &conn_id)?;
    let end = ConnectionEnd {
        client_id: client_id.to_owned(),
        versions,
        state: State::Init as i32,
        counterparty: Some(counterparty.clone()),
        delay_period,
    };
    set_connection(ctx, &conn_id, &end)?;
    ctx.events.emit_all(conn_events(
        "connection_open_init",
        vec![
            ("connection_id", conn_id.clone()),
            ("client_id", client_id.to_owned()),
            ("counterparty_client_id", counterparty.client_id.clone()),
        ],
    ));
    Ok(conn_id)
}

/// `ConnOpenTry`.
#[allow(clippy::too_many_arguments)]
pub fn conn_open_try(
    ctx: &mut Context<'_>,
    counterparty: &Counterparty,
    delay_period: u64,
    client_id: &str,
    counterparty_versions: &[Version],
    init_proof: &[u8],
    proof_height: &Height,
) -> Result<String> {
    let conn_id = generate_connection_identifier(ctx)?;
    let expected = ConnectionEnd {
        client_id: counterparty.client_id.clone(),
        versions: counterparty_versions.to_vec(),
        state: State::Init as i32,
        counterparty: Some(Counterparty {
            client_id: client_id.to_owned(),
            connection_id: String::new(),
            prefix: Some(commitment_prefix()),
        }),
        delay_period,
    };
    let version = pick_version(&get_compatible_versions(), counterparty_versions)?;
    let end = ConnectionEnd {
        client_id: client_id.to_owned(),
        versions: vec![version],
        state: State::Tryopen as i32,
        counterparty: Some(counterparty.clone()),
        delay_period,
    };
    verify_connection_state(
        ctx,
        &end,
        proof_height,
        init_proof,
        &counterparty.connection_id,
        &expected,
    )?;
    add_connection_to_client(ctx, client_id, &conn_id).map_err(|e| {
        e.wrap(format!(
            "failed to add connection with ID {conn_id} to client with ID {client_id}"
        ))
    })?;
    set_connection(ctx, &conn_id, &end)?;
    ctx.events.emit_all(conn_events(
        "connection_open_try",
        vec![
            ("connection_id", conn_id.clone()),
            ("client_id", client_id.to_owned()),
            ("counterparty_client_id", counterparty.client_id.clone()),
            (
                "counterparty_connection_id",
                counterparty.connection_id.clone(),
            ),
        ],
    ));
    Ok(conn_id)
}

/// `ConnOpenAck`.
pub fn conn_open_ack(
    ctx: &mut Context<'_>,
    conn_id: &str,
    version: &Version,
    counterparty_conn_id: &str,
    try_proof: &[u8],
    proof_height: &Height,
) -> Result<()> {
    let mut conn =
        get_connection(ctx, conn_id)?.ok_or_else(|| cerr::connection_not_found(conn_id))?;
    if conn.state != State::Init as i32 {
        return Err(cerr::invalid_connection_state(format!(
            "connection state is not INIT (got {})",
            state_name(conn.state)
        )));
    }
    if !is_supported_version(&conn.versions, version) {
        return Err(cerr::invalid_connection_state(format!(
            "the counterparty selected version {} is not supported by versions selected on INIT",
            go_version_string(version)
        )));
    }
    let cp = conn.counterparty.clone().unwrap_or_default();
    let expected = ConnectionEnd {
        client_id: cp.client_id.clone(),
        versions: vec![version.clone()],
        state: State::Tryopen as i32,
        counterparty: Some(Counterparty {
            client_id: conn.client_id.clone(),
            connection_id: conn_id.to_owned(),
            prefix: Some(commitment_prefix()),
        }),
        delay_period: conn.delay_period,
    };
    verify_connection_state(
        ctx,
        &conn,
        proof_height,
        try_proof,
        counterparty_conn_id,
        &expected,
    )?;
    conn.state = State::Open as i32;
    conn.versions = vec![version.clone()];
    let mut cp = cp;
    counterparty_conn_id.clone_into(&mut cp.connection_id);
    conn.counterparty = Some(cp.clone());
    set_connection(ctx, conn_id, &conn)?;
    ctx.events.emit_all(conn_events(
        "connection_open_ack",
        vec![
            ("connection_id", conn_id.to_owned()),
            ("client_id", conn.client_id.clone()),
            ("counterparty_client_id", cp.client_id.clone()),
            ("counterparty_connection_id", cp.connection_id),
        ],
    ));
    Ok(())
}

/// `ConnOpenConfirm`.
pub fn conn_open_confirm(
    ctx: &mut Context<'_>,
    conn_id: &str,
    ack_proof: &[u8],
    proof_height: &Height,
) -> Result<()> {
    let mut conn =
        get_connection(ctx, conn_id)?.ok_or_else(|| cerr::connection_not_found(conn_id))?;
    if conn.state != State::Tryopen as i32 {
        return Err(cerr::invalid_connection_state(format!(
            "connection state is not TRYOPEN (got {})",
            state_name(conn.state)
        )));
    }
    let cp = conn.counterparty.clone().unwrap_or_default();
    let expected = ConnectionEnd {
        client_id: cp.client_id.clone(),
        versions: conn.versions.clone(),
        state: State::Open as i32,
        counterparty: Some(Counterparty {
            client_id: conn.client_id.clone(),
            connection_id: conn_id.to_owned(),
            prefix: Some(commitment_prefix()),
        }),
        delay_period: conn.delay_period,
    };
    verify_connection_state(
        ctx,
        &conn,
        proof_height,
        ack_proof,
        &cp.connection_id,
        &expected,
    )?;
    conn.state = State::Open as i32;
    set_connection(ctx, conn_id, &conn)?;
    ctx.events.emit_all(conn_events(
        "connection_open_confirm",
        vec![
            ("connection_id", conn_id.to_owned()),
            ("client_id", conn.client_id.clone()),
            ("counterparty_client_id", cp.client_id.clone()),
            ("counterparty_connection_id", cp.connection_id),
        ],
    ));
    Ok(())
}

// ---------------------------------------------------------------------------
// Verification (`keeper/verify.go`)
// ---------------------------------------------------------------------------

/// `getClientStateAndVerificationStore` + `GetClientStatus` check.
fn verification_client(ctx: &mut Context<'_>, conn: &ConnectionEnd) -> Result<client::ClientState> {
    let client_id = &conn.client_id;
    let cs = client::client_state(ctx, client_id)?
        .ok_or_else(|| clerr::client_not_found(client_id.clone()))?;
    let status = client::client_status(ctx, client_id, &cs)?;
    if status != client::Status::Active {
        return Err(clerr::client_not_active(format!(
            "client ({client_id}) status is {}",
            status.as_str()
        )));
    }
    Ok(cs)
}

/// `getBlockDelay`: `ceil(delay / max_expected_time_per_block)`.
fn get_block_delay(ctx: &mut Context<'_>, conn: &ConnectionEnd) -> Result<u64> {
    let expected = get_params(ctx)?.max_expected_time_per_block;
    if expected == 0 {
        return Ok(0);
    }
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    Ok((conn.delay_period as f64 / expected as f64).ceil() as u64)
}

/// The proof kinds verified against a connection's client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Handshake state (no delay period).
    State,
    /// Packet data (delay period applies).
    Packet,
}

/// Verifies `value` (or absence with `None`) at `path` on the counterparty
/// under the connection's client; `wrap` names the failure like Go.
#[allow(clippy::too_many_arguments)]
pub fn verify(
    ctx: &mut Context<'_>,
    conn: &ConnectionEnd,
    h: &Height,
    proof: &[u8],
    path: &[String],
    value: Option<&[u8]>,
    kind: Kind,
    wrap: &str,
) -> Result<()> {
    verify_with(ctx, conn, h, proof, path, value, kind, |e, client_id| {
        e.wrap(format!("{wrap} ({client_id})"))
    })
}

/// [`verify`] with a custom wrapping of the verification error.
#[allow(clippy::too_many_arguments)]
pub fn verify_with(
    ctx: &mut Context<'_>,
    conn: &ConnectionEnd,
    h: &Height,
    proof: &[u8],
    path: &[String],
    value: Option<&[u8]>,
    kind: Kind,
    wrap: impl FnOnce(SdkError, &str) -> SdkError,
) -> Result<()> {
    let cs = verification_client(ctx, conn)?;
    let (delay_time, delay_block) = match kind {
        Kind::State => (0, 0),
        Kind::Packet => (conn.delay_period, get_block_delay(ctx, conn)?),
    };
    let cp = conn.counterparty.clone().unwrap_or_default();
    let merkle_path = crate::commitment::apply_prefix(cp.prefix.as_ref(), path)?;
    client::verify_membership(
        ctx,
        &conn.client_id,
        &cs,
        h,
        delay_time,
        delay_block,
        proof,
        &merkle_path,
        value,
    )
    .map_err(|e| wrap(e, &conn.client_id))
}

/// `VerifyConnectionState`.
pub fn verify_connection_state(
    ctx: &mut Context<'_>,
    conn: &ConnectionEnd,
    h: &Height,
    proof: &[u8],
    counterparty_conn_id: &str,
    expected: &ConnectionEnd,
) -> Result<()> {
    let bz = normalize_connection(expected.clone()).encode_to_vec();
    verify(
        ctx,
        conn,
        h,
        proof,
        &[String::from_utf8_lossy(&host::connection_key(counterparty_conn_id)).into_owned()],
        Some(&bz),
        Kind::State,
        "failed connection state verification for client",
    )
}

/// Fills the non-nullable `prefix` of a counterparty.
#[must_use]
pub fn normalize_counterparty(mut c: Counterparty) -> Counterparty {
    if c.prefix.is_none() {
        c.prefix = Some(MerklePrefix::default());
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ibc-go v8.7.2 `PickVersion` prints both lists with `%v` (Go's text
    /// from deliver-diff `zkism-forwarding` seed 5083, a failed proposal's
    /// `failed_reason`).
    #[test]
    fn pick_version_error_is_go_s() {
        let cp = |id: &str, f: &[&str]| Version {
            identifier: id.into(),
            features: f.iter().map(|s| (*s).to_owned()).collect(),
        };
        let err = pick_version(
            &get_compatible_versions(),
            &[cp(
                "celestia1esdcnqdhef5kl6t52cns557lk453nxk7jmy8gn",
                &["a"],
            )],
        )
        .unwrap_err();
        assert_eq!(
            err.log,
            "failed to find a matching counterparty version \
             ([identifier:\"celestia1esdcnqdhef5kl6t52cns557lk453nxk7jmy8gn\" features:\"a\" ]) \
             from the supported version list \
             ([identifier:\"1\" features:\"ORDER_ORDERED\" features:\"ORDER_UNORDERED\" ]): \
             connection version negotiation failed"
        );
        let err =
            pick_version(&get_compatible_versions(), &[cp("", &[]), cp("1", &[])]).unwrap_err();
        // `ConnOpenAck`'s `%s` of the selected `*Version`.
        assert_eq!(
            go_version_string(&cp("1", &["ORDER_ORDERED", "x\"y"])),
            "identifier:\"1\" features:\"ORDER_ORDERED\" features:\"x\\\"y\" "
        );
        // An empty `Version` prints as an empty string.
        assert!(err.log.starts_with(
            "failed to find a matching counterparty version ([ identifier:\"1\" ]) from"
        ));
    }
}
