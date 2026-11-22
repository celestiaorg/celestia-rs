//! DA service identity, readiness and token administration on either role.
use super::auth::Auth;
use anyhow::Result;
use serde_json::{Value, json};

pub(super) fn permission(method: &str) -> Option<&'static str> {
    match method {
        "node.Ready" => Some("read"),
        "node.Info"
        | "node.AuthNew"
        | "node.AuthNewWithExpiry"
        | "node.LogLevelSet"
        | "node.AuthVerify"
        | "node.AuthRevoke" => Some("admin"),
        _ => None,
    }
}

pub(super) fn call(auth: &Auth, role: &str, method: &str, args: &[Value]) -> Result<Value> {
    match (method, args) {
        ("node.Ready", []) => Ok(Value::Bool(true)),
        ("node.Info", []) => Ok(json!({"type": role, "api_version":env!("CARGO_PKG_VERSION")})),
        ("node.LogLevelSet", [Value::String(component), Value::String(level)]) => {
            crate::ops::da_logging::set(component, level)?;
            Ok(Value::Null)
        }
        ("node.Ready" | "node.Info" | "node.LogLevelSet", _) => anyhow::bail!("invalid parameters"),
        _ => auth.admin(method, args),
    }
}
