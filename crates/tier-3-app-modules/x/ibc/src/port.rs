//! ICS-05 port keeper (`core/05-port`): binds ports to capabilities owned
//! by the `ibc` scoped keeper.

use celestia_proto::capability::v1::Capability;
use sdk::{Context, Result, SdkError};
use x_capability::ScopedKeeper;

use crate::host;

/// Sub-module name.
pub const SUB_MODULE_NAME: &str = "port";

/// The `ibc` module's scoped capability keeper (`ScopedIBCKeeper`).
pub const SCOPED_KEEPER: ScopedKeeper = ScopedKeeper::new(crate::MODULE_NAME);

/// `ErrPortExists` (`port` codespace, code 2).
#[must_use]
pub fn err_port_exists(log: &str) -> SdkError {
    SdkError::new("port", 2, format!("{log}: port is already binded"))
}

/// `IsBound`.
pub fn is_bound(ctx: &mut Context<'_>, port_id: &str) -> Result<bool> {
    Ok(SCOPED_KEEPER
        .get_capability(ctx, &host::port_path(port_id))?
        .is_some())
}

/// `BindPort`: creates the port capability. Go panics on an invalid or
/// already bound port; this returns the error.
pub fn bind_port(ctx: &mut Context<'_>, port_id: &str) -> Result<Capability> {
    host::port_identifier_validator(port_id)?;
    if is_bound(ctx, port_id)? {
        return Err(err_port_exists(&format!("port {port_id} is already bound")));
    }
    SCOPED_KEEPER.new_capability(ctx, &host::port_path(port_id))
}
