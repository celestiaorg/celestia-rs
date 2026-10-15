//! Port of cosmos-sdk `codec/unknownproto` (celestia fork v0.52.12).
//!
//! Walks the wire format against the message schema, rejecting unknown
//! fields (non-critical ones, i.e. field numbers with bit 11 set, are allowed
//! when requested) and wire types that cannot encode the declared field type,
//! recursing into nested messages and into `Any` values resolved by type URL
//! through the interface registry.

use celestia_proto::wire_table::{FieldInfo, MessageInfo, ty};

use crate::gogo::{self, GoError, GoResult};
use crate::protowire::{self, WireType};
use crate::registry;

const BIT11_NON_CRITICAL: i64 = 1 << 10;

/// `RejectUnknownFields`: returns whether non-critical unknown fields were
/// seen, and the error (if any) – Go returns both.
pub(crate) fn reject_unknown_fields(
    bz: &[u8],
    msg: &'static MessageInfo,
    allow_unknown_non_criticals: bool,
) -> (bool, GoResult<()>) {
    let mut has = false;
    let r = walk(
        bz,
        msg,
        allow_unknown_non_criticals,
        protowire::DEFAULT_RECURSION_LIMIT,
        &mut has,
    );
    (has, r)
}

/// `canEncodeType`.
fn can_encode_type(wt: WireType, t: u8) -> bool {
    match wt {
        protowire::VARINT => matches!(
            t,
            ty::INT32
                | ty::INT64
                | ty::UINT32
                | ty::UINT64
                | ty::SINT32
                | ty::SINT64
                | ty::BOOL
                | ty::ENUM
        ),
        protowire::FIXED64 => matches!(t, ty::FIXED64 | ty::SFIXED64 | ty::DOUBLE),
        protowire::BYTES => !matches!(t, ty::FIXED32 | ty::SFIXED32 | ty::FLOAT | ty::GROUP),
        protowire::START_GROUP | protowire::END_GROUP => t == ty::GROUP,
        protowire::FIXED32 => matches!(t, ty::FIXED32 | ty::SFIXED32 | ty::FLOAT),
        _ => false,
    }
}

/// `isScalar` (gogo descriptor helper): numeric types and enums.
fn is_scalar(f: &FieldInfo) -> bool {
    !matches!(f.ty, ty::STRING | ty::BYTES | ty::MESSAGE | ty::GROUP)
}

fn walk(
    mut bz: &[u8],
    msg: &'static MessageInfo,
    allow: bool,
    recursion_limit: i32,
    has: &mut bool,
) -> GoResult<()> {
    if bz.is_empty() {
        return Ok(());
    }
    if recursion_limit == 0 {
        return Err(GoError::new("recursion limit reached"));
    }
    while !bz.is_empty() {
        let (tag_num, wire_type, m) = protowire::consume_tag(bz);
        if m < 0 {
            return Err(GoError::new("invalid length"));
        }
        let field = u32::try_from(tag_num)
            .ok()
            .filter(|n| registry::field_exists(msg.name, *n))
            .and_then(|n| msg.field(n));
        if let Some(f) = field {
            {
                if !can_encode_type(wire_type, f.ty) {
                    return Err(GoError::new(format!(
                        "Mismatched {:?}: {{TagNum: {tag_num}, GotWireType: {wire_type}}}",
                        msg.name
                    )));
                }
            }
        } else {
            {
                let critical = tag_num & BIT11_NON_CRITICAL == 0;
                if !critical {
                    *has = true;
                }
                if critical || !allow {
                    return Err(GoError::new(format!(
                        "errUnknownField {:?}: {{TagNum: {tag_num}, WireType:{wire_type}}}",
                        msg.name
                    )));
                }
            }
        }
        bz = &bz[m as usize..];
        let n = protowire::consume_field_value(tag_num, wire_type, bz);
        if n < 0 {
            return Err(GoError::new(format!(
                "could not consume field value for tagNum: {tag_num}; {}",
                protowire::parse_error(n)
            )));
        }
        let mut field_bytes = &bz[..n as usize];
        bz = &bz[n as usize..];

        let Some(f) = field else { continue };
        if is_scalar(f) || f.type_name.is_empty() {
            continue;
        }
        // consume length prefix of nested message
        let (_, o) = protowire::consume_varint(field_bytes);
        field_bytes = &field_bytes[o.max(0) as usize..];

        let nested: &'static MessageInfo = if f.type_name == "google.protobuf.Any" {
            let any_info = gogo::info("google.protobuf.Any")?;
            walk(field_bytes, any_info, allow, recursion_limit - 1, has)?;
            let any = gogo::unmarshal("google.protobuf.Any", field_bytes)?;
            let type_url = any.bytes(1);
            let resolved = registry::resolve(type_url).ok_or_else(|| {
                GoError::new(format!(
                    "unable to resolve type URL {}",
                    String::from_utf8_lossy(type_url)
                ))
            })?;
            let value = any.bytes(2).to_vec();
            walk(&value, resolved, allow, recursion_limit - 1, has)?;
            continue;
        } else {
            gogo::info(f.type_name)?
        };
        walk(field_bytes, nested, allow, recursion_limit - 1, has)?;
    }
    Ok(())
}
