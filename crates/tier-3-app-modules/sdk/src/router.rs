//! The message service router as modules see it (`baseapp.MessageRouter`).
//!
//! Modules that execute nested messages (gov proposals, authz `MsgExec`)
//! take a `&dyn MsgRouter`; the app implements it over its router.

use celestia_proto::Any;

use crate::{Context, Result};

/// `baseapp.MessageRouter`.
pub trait MsgRouter {
    /// `router.Handler(msg) != nil` / `HandlerByTypeURL(url) != nil`.
    fn has_handler(&self, type_url: &str) -> bool;
    /// `ValidateBasic` of messages that implement `sdk.HasValidateBasic`
    /// (others: `Ok`).
    fn validate_basic(&self, msg: &Any) -> Result<()>;
    /// The registered handler: `ValidateBasic`, circuit breaker, then the
    /// msg server. Returns the packed response; events are emitted on `ctx`.
    fn handle(&self, ctx: &mut Context<'_>, msg: &Any) -> Result<Any>;
}
