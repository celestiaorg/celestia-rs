//! Compact protobuf schema for wire-level validation.
//!
//! Generated from the same descriptors as the prost bindings: for every
//! message, its fields with descriptor type, label, nested type name and the
//! gogoproto options that change how Go decodes the field (`nullable`,
//! `customtype`, `stdtime`, `stdduration`). Used to port Go's
//! `codec/unknownproto` and gogoproto-generated `Unmarshal` behavior
//! generically (see the `txdecode` crate).

/// `FieldDescriptorProto.Type` values.
pub mod ty {
    pub const DOUBLE: u8 = 1;
    pub const FLOAT: u8 = 2;
    pub const INT64: u8 = 3;
    pub const UINT64: u8 = 4;
    pub const INT32: u8 = 5;
    pub const FIXED64: u8 = 6;
    pub const FIXED32: u8 = 7;
    pub const BOOL: u8 = 8;
    pub const STRING: u8 = 9;
    pub const GROUP: u8 = 10;
    pub const MESSAGE: u8 = 11;
    pub const BYTES: u8 = 12;
    pub const UINT32: u8 = 13;
    pub const ENUM: u8 = 14;
    pub const SFIXED32: u8 = 15;
    pub const SFIXED64: u8 = 16;
    pub const SINT32: u8 = 17;
    pub const SINT64: u8 = 18;
}

/// One field of a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldInfo {
    /// Field number.
    pub number: u32,
    /// Descriptor type (see [`ty`]).
    pub ty: u8,
    /// `repeated` (including map fields).
    pub repeated: bool,
    /// Packed encoding for repeated scalars.
    pub packed: bool,
    /// Full name (no leading dot) of the message or enum type; empty for
    /// scalars.
    pub type_name: &'static str,
    /// `(gogoproto.customtype)`, empty if unset.
    pub custom: &'static str,
    /// `(gogoproto.stdtime) = true`.
    pub std_time: bool,
    /// `(gogoproto.stdduration) = true`.
    pub std_duration: bool,
    /// `(gogoproto.nullable)` (default `true`).
    pub nullable: bool,
    /// Index of the (non-synthetic) oneof this field belongs to, or -1.
    pub oneof: i32,
}

impl FieldInfo {
    /// Whether the field holds a `google.protobuf.Any`.
    #[must_use]
    pub fn is_any(&self) -> bool {
        self.ty == ty::MESSAGE && self.type_name == "google.protobuf.Any"
    }
}

/// One message type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageInfo {
    /// Full name without leading dot, e.g. `cosmos.tx.v1beta1.TxBody`.
    pub name: &'static str,
    /// Synthesized map entry type.
    pub map_entry: bool,
    /// Fields sorted by number.
    pub fields: &'static [FieldInfo],
}

impl MessageInfo {
    /// The field with `number`.
    #[must_use]
    pub fn field(&self, number: u32) -> Option<&'static FieldInfo> {
        self.fields
            .binary_search_by_key(&number, |f| f.number)
            .ok()
            .map(|i| &self.fields[i])
    }
}

use MessageInfo as M;

/// Compact constructor used by the generated table. `flags`: bit 0
/// repeated, 1 packed, 2 stdtime, 3 stdduration, 4 nullable.
const fn f(
    number: u32,
    ty: u8,
    flags: u8,
    oneof: i32,
    type_name: &'static str,
    custom: &'static str,
) -> FieldInfo {
    FieldInfo {
        number,
        ty,
        repeated: flags & 1 != 0,
        packed: flags & 2 != 0,
        type_name,
        custom,
        std_time: flags & 4 != 0,
        std_duration: flags & 8 != 0,
        nullable: flags & 16 != 0,
        oneof,
    }
}

include!("gen/wire_table.rs");

/// Looks up a message by full name (no leading dot).
#[must_use]
pub fn message(name: &str) -> Option<&'static MessageInfo> {
    MESSAGES
        .binary_search_by(|m| m.name.cmp(name))
        .ok()
        .map(|i| &MESSAGES[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tx_body_schema() {
        let body = message("cosmos.tx.v1beta1.TxBody").unwrap();
        let msgs = body.field(1).unwrap();
        assert!(msgs.repeated && msgs.is_any());
        assert_eq!(body.field(2).unwrap().ty, ty::STRING);
        assert!(body.field(1023).unwrap().is_any());
        assert!(body.field(4).is_none());
        let coin = message("cosmos.base.v1beta1.Coin").unwrap();
        let amount = coin.field(2).unwrap();
        assert_eq!(amount.custom, "cosmossdk.io/math.Int");
        assert!(!amount.nullable);
        assert!(MESSAGES.windows(2).all(|w| w[0].name < w[1].name));
    }
}
