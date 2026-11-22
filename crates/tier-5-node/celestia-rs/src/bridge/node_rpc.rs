//! DA service identity, readiness and token administration on either role.
use super::auth::Auth;
use anyhow::Result;
use serde_json::{Value, json};

pub(super) fn permission(method: &str) -> Option<&'static str> {
    match method {
        "node.Ready" => Some("read"),
        "node.Info"
        | "node.LogLevelSet"
        | "node.AuthNew"
        | "node.AuthNewWithExpiry"
        | "node.AuthVerify"
        | "node.AuthRevoke" => Some("admin"),
        _ => None,
    }
}

pub(super) fn call(auth: &Auth, role: &str, method: &str, args: &[Value]) -> Result<Value> {
    tracing::debug!(method, "DA node RPC");
    match (method, args) {
        ("node.Ready", []) => Ok(Value::Bool(true)),
        ("node.Info", []) => Ok(json!({"type": role, "api_version":env!("CARGO_PKG_VERSION")})),
        ("node.LogLevelSet", [Value::String(name), Value::String(level)]) => {
            crate::ops::da_logging::set_level(name, level)?;
            Ok(Value::Null)
        }
        ("node.Ready" | "node.Info" | "node.LogLevelSet", _) => anyhow::bail!("invalid parameters"),
        _ => auth.admin(method, args),
    }
}
