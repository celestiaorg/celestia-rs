//! Query service method table and message field schema.
//!
//! Generated from the same descriptors as the prost bindings: every unary
//! method of every non-`Msg` service (full gRPC path, request and response
//! type names, `google.api.http` bindings) and, for every message, its
//! fields by name. The gRPC server routes by [`METHODS`]; the REST gateway
//! derives its routes from the HTTP bindings and parses/renders JSON with
//! [`SCHEMA`] — no route is written by hand.

/// A `google.api.http` binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpRule {
    /// HTTP verb (`GET`, `POST`, …).
    pub method: &'static str,
    /// URL template, e.g. `/cosmos/bank/v1beta1/balances/{address}`.
    pub pattern: &'static str,
    /// Request body field (`*` for the whole request), empty for none.
    pub body: &'static str,
}

/// A unary query method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Method {
    /// Full gRPC path, e.g. `/cosmos.bank.v1beta1.Query/Balance`.
    pub path: &'static str,
    /// Request message full name.
    pub input: &'static str,
    /// Response message full name.
    pub output: &'static str,
    /// Declaration order (descriptor file, service, method): the order in
    /// which grpc-gateway registers the method's routes.
    pub order: u32,
    /// HTTP bindings (primary first).
    pub http: &'static [HttpRule],
}

/// One field of a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    /// Proto field name.
    pub name: &'static str,
    /// JSON (lowerCamelCase) name.
    pub json_name: &'static str,
    /// Field number.
    pub number: u32,
    /// Descriptor type (see [`crate::wire_table::ty`]).
    pub ty: u8,
    /// `repeated` or map.
    pub repeated: bool,
    /// Message or enum full name; empty for scalars.
    pub type_name: &'static str,
    /// 0: plain, 1: proto3 `optional`, 2: member of a real oneof.
    pub oneof: u8,
}

/// A message schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Message {
    /// Full name.
    pub name: &'static str,
    /// Synthesized map entry.
    pub map_entry: bool,
    /// Fields sorted by number.
    pub fields: &'static [Field],
}

impl Message {
    /// The field named `name` (proto or JSON name).
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&'static Field> {
        self.fields
            .iter()
            .find(|f| f.name == name || f.json_name == name)
    }
}

use Field as F;
use HttpRule as H;
use Message as Msg;

include!("gen/services.rs");

/// The method with gRPC path `path`.
#[must_use]
pub fn method(path: &str) -> Option<&'static Method> {
    METHODS
        .binary_search_by(|m| m.path.cmp(path))
        .ok()
        .map(|i| &METHODS[i])
}

/// The schema of message `name` (full name, no leading dot).
#[must_use]
pub fn message(name: &str) -> Option<&'static Message> {
    SCHEMA
        .binary_search_by(|m| m.name.cmp(name))
        .ok()
        .map(|i| &SCHEMA[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bank_balance_binding() {
        let m = method("/cosmos.bank.v1beta1.Query/Balance").unwrap();
        assert_eq!(m.input, "cosmos.bank.v1beta1.QueryBalanceRequest");
        assert_eq!(m.http[0].method, "GET");
        assert_eq!(
            m.http[0].pattern,
            "/cosmos/bank/v1beta1/balances/{address}/by_denom"
        );
        assert!(method("/cosmos.bank.v1beta1.Msg/Send").is_none());
        let req = message("cosmos.bank.v1beta1.QueryAllBalancesRequest").unwrap();
        assert_eq!(req.field("pagination").unwrap().number, 2);
        assert_eq!(req.field("resolveDenom").unwrap().name, "resolve_denom");
        assert!(METHODS.windows(2).all(|w| w[0].path < w[1].path));
    }
}
