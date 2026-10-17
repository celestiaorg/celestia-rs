//! Systematic wire-level protobuf mutations (`fuzz-diff wire`,
//! `DELIVER_WIRE`).
//!
//! [`crate::mutate`] samples random edits; this module *enumerates* every
//! structural rewrite of a valid encoded message that protobuf decoders
//! are known to read differently, at every nesting level (embedded
//! messages, `stdtime` / `stdduration` fields, the values of `Any`s
//! resolved through the schema), so the class is exhausted per message
//! type rather than sampled:
//!
//! | class | rewrite |
//! |---|---|
//! | `dup_same` / `dup_last` / `dup_first` | a singular field repeated: the same value, another value after, another value before (scalars, strings, bytes, customtypes) |
//! | `msg_empty_last` / `msg_split` / `msg_part_last` / `msg_alt_last` | a singular embedded message (`stdtime`, `stdduration`, `Any`, customtype messages included) repeated: an empty occurrence after, its fields split over two occurrences, a part of it after, a changed copy after (gogoproto merges messages, overwrites `stdtime` / `stdduration`) |
//! | `packed` / `unpacked` / `packed_mixed` / `packed_empty` / `repeated_interleaved` / `packed_long10` / `packed_nonmin` / `unpacked_long10` | repeated scalars packed, unpacked, both in one message, an empty packed run, elements interleaved with other fields; varint elements of a packed run as 10-byte varints with high bits or non-minimal, unpacked elements as 10-byte varints |
//! | `reorder_rev` / `reorder_rot` | fields out of order |
//! | `unknown` | an unknown field: critical numbers, non-critical (bit 11) numbers, 19000-19999, 2^29-1, 2^29, 2^31-1, keys past 2^32 (gogoproto truncates the number to `int32`), each wire type, at the start and end |
//! | `wrong_wt` | a known field with each other wire type |
//! | `zero_len` / `default_explicit` / `drop` | an embedded message emptied; absent fields encoded with their default value; a field removed |
//! | `nonmin_tag` / `nonmin_val` / `nonmin_len` / `long10_tag` / `long10_val` / `long10_len` | non-minimal varints (`0x80 0x00` padding) and 10-byte varints with high bits in the last byte, in keys, values and length prefixes |
//! | `group` / `group_mismatch` / `group_lone_end` / `group_open` / `group_known` | deprecated wire types 3 / 4: a group, a group whose end tag names another field, an end tag alone, a start tag alone, a group on a known field number |
//! | `field0` / `wt67` | field number 0, wire types 6 and 7 |
//! | `oneof_two` / `oneof_two_first` | two members of one oneof |
//! | `any_url_*` / `any_value_*` / `any_split` | `Any` edge cases: type URL without `/`, with a host, `//`, empty, trailing bytes; empty value, trailing garbage, truncated, type URL and value in separate occurrences |
//!
//! No message reachable from a tx has a map field (the only map in the
//! schema is PFM's genesis `InFlightPacketsEntry`), so maps are not a
//! class. Each [`Mutation`] names its class and the field path, so the
//! drivers can count volumes per class and level.

use celestia_proto::wire_table::{self, FieldInfo, MessageInfo, ty};

use crate::mutate::{Field, parse, put_varint};

/// One rewritten message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mutation {
    /// The mutation class (see the module table).
    pub class: &'static str,
    /// Field path of the rewritten node (`3.1@cosmos.bank.v1beta1.MsgSend.2`).
    pub path: String,
    /// The whole top-level message after the rewrite.
    pub bytes: Vec<u8>,
}

/// Maximum nesting depth the enumeration descends to.
pub const MAX_DEPTH: usize = 5;

/// The enumeration of one message stops descending past this many
/// mutations (messages with long repeated fields).
pub const MAX_MUTATIONS: usize = 6000;

/// The schema name of a type URL (the part after the last `/`).
#[must_use]
pub fn type_name(type_url: &[u8]) -> Option<&'static MessageInfo> {
    let s = std::str::from_utf8(type_url).ok()?;
    wire_table::message(s.rsplit('/').next()?)
}

/// Every structural rewrite of `b` (a message of type `name`).
#[must_use]
pub fn enumerate(name: &str, b: &[u8]) -> Vec<Mutation> {
    let mut out = Vec::new();
    if let Some(info) = wire_table::message(name) {
        let wrap = |x: Vec<u8>| x;
        walk(info, b, 0, name, &wrap, &mut out);
    }
    out
}

fn tag(num: u64, wt: u8) -> Vec<u8> {
    let mut v = Vec::new();
    put_varint(&mut v, (num << 3) | u64::from(wt));
    v
}

fn varint(v: u64) -> Vec<u8> {
    let mut o = Vec::new();
    put_varint(&mut o, v);
    o
}

/// `v` with `extra` redundant continuation bytes (`0x80 ... 0x00`).
fn varint_nonmin(v: u64, extra: usize) -> Vec<u8> {
    let mut o = varint(v);
    if let Some(l) = o.last_mut() {
        *l |= 0x80;
    }
    o.extend(std::iter::repeat_n(0x80, extra.saturating_sub(1)));
    o.push(0x00);
    o
}

/// `v` as 10 bytes whose last byte also carries `hi` above its low bit
/// (gogoproto drops the bits past 64; protobuf-go rejects the varint).
fn varint_long10(v: u64, hi: u8) -> Vec<u8> {
    let mut o = Vec::with_capacity(10);
    for k in 0..9 {
        o.push(((v >> (7 * k)) as u8 & 0x7f) | 0x80);
    }
    o.push(((v >> 63) as u8 & 1) | (hi << 1));
    o
}

fn field_bytes(f: &Field) -> Vec<u8> {
    let mut o = tag(f.num, f.wt);
    if f.wt == 2 {
        put_varint(&mut o, f.val.len() as u64);
    }
    o.extend_from_slice(&f.val);
    o
}

fn len_field(num: u64, val: &[u8]) -> Vec<u8> {
    field_bytes(&Field {
        num,
        wt: 2,
        val: val.to_vec(),
    })
}

/// The encoding of `fs` with field `i` replaced by `raw` (`None`: removed).
fn with_replaced(fs: &[Field], i: usize, raw: Option<&[u8]>) -> Vec<u8> {
    let mut o = Vec::new();
    for (k, f) in fs.iter().enumerate() {
        if k == i {
            if let Some(r) = raw {
                o.extend_from_slice(r);
            }
        } else {
            o.extend(field_bytes(f));
        }
    }
    o
}

/// The encoding of `fs` with `raw` inserted before field `at` (`fs.len()`:
/// at the end).
fn with_inserted(fs: &[Field], at: usize, raw: &[u8]) -> Vec<u8> {
    let mut o = Vec::new();
    for (k, f) in fs.iter().enumerate() {
        if k == at {
            o.extend_from_slice(raw);
        }
        o.extend(field_bytes(f));
    }
    if at >= fs.len() {
        o.extend_from_slice(raw);
    }
    o
}

fn is_varint(t: u8) -> bool {
    matches!(
        t,
        ty::INT32
            | ty::INT64
            | ty::UINT32
            | ty::UINT64
            | ty::SINT32
            | ty::SINT64
            | ty::BOOL
            | ty::ENUM
    )
}

fn is_fixed32(t: u8) -> bool {
    matches!(t, ty::FIXED32 | ty::SFIXED32 | ty::FLOAT)
}

fn is_fixed64(t: u8) -> bool {
    matches!(t, ty::FIXED64 | ty::SFIXED64 | ty::DOUBLE)
}

fn is_numeric(t: u8) -> bool {
    is_varint(t) || is_fixed32(t) || is_fixed64(t)
}

/// The wire type a field of `f`'s type is encoded with (unpacked).
fn natural_wt(f: &FieldInfo) -> u8 {
    if is_varint(f.ty) {
        0
    } else if is_fixed32(f.ty) {
        5
    } else if is_fixed64(f.ty) {
        1
    } else {
        2
    }
}

/// Another value of a scalar field (raw value bytes for its wire type).
fn scalar_alts(f: &FieldInfo, cur: &[u8]) -> Vec<Vec<u8>> {
    let mut v: Vec<Vec<u8>> = if is_varint(f.ty) {
        let x = read_varint(cur).unwrap_or(0);
        vec![
            varint(0),
            varint(1),
            varint(x.wrapping_add(1)),
            varint(u64::MAX),
        ]
    } else if is_fixed32(f.ty) {
        vec![vec![0; 4], vec![1, 0, 0, 0], vec![0xff; 4]]
    } else if is_fixed64(f.ty) {
        vec![vec![0; 8], vec![1, 0, 0, 0, 0, 0, 0, 0], vec![0xff; 8]]
    } else if f.custom.ends_with("math.Int") {
        vec![
            b"".to_vec(),
            b"0".to_vec(),
            b"7".to_vec(),
            b"-1".to_vec(),
            b"+1".to_vec(),
        ]
    } else if f.custom.ends_with("math.LegacyDec") {
        vec![
            b"".to_vec(),
            b"0".to_vec(),
            b"1000000000000000000".to_vec(),
            b"500000000000000000".to_vec(),
        ]
    } else if f.custom.ends_with("HexAddress") {
        vec![Vec::new(), [0u8; 32].to_vec(), {
            let mut a = [0u8; 32];
            a[31] = 1;
            a.to_vec()
        }]
    } else if f.ty == ty::BYTES {
        vec![Vec::new(), vec![1], cur.iter().rev().copied().collect()]
    } else {
        let mut other = cur.to_vec();
        other.push(b'x');
        vec![Vec::new(), b"x".to_vec(), other]
    };
    v.retain(|x| x.as_slice() != cur);
    v
}

fn read_varint(b: &[u8]) -> Option<u64> {
    let mut v = 0u64;
    for (i, &x) in b.iter().enumerate().take(10) {
        v |= u64::from(x & 0x7f).checked_shl(7 * i as u32).unwrap_or(0);
        if x < 0x80 {
            return Some(v);
        }
    }
    None
}

/// A field's default value encoded explicitly (for an absent field).
fn default_encodings(f: &FieldInfo) -> Vec<Vec<u8>> {
    let n = u64::from(f.number);
    if is_numeric(f.ty) {
        let mut o = tag(n, natural_wt(f));
        o.extend(match natural_wt(f) {
            0 => vec![0],
            5 => vec![0; 4],
            _ => vec![0; 8],
        });
        let mut v = vec![o];
        if f.repeated {
            // An empty packed run.
            v.push(len_field(n, &[]));
        }
        v
    } else if f.ty == ty::MESSAGE && !f.std_time && !f.std_duration {
        vec![len_field(n, &[])]
    } else if f.custom.ends_with("math.Int") || f.custom.ends_with("math.LegacyDec") {
        vec![len_field(n, b""), len_field(n, b"0")]
    } else {
        vec![len_field(n, &[])]
    }
}

/// Values for another wire type of a field (raw, tag included).
fn wrong_wire(num: u64, natural: u8, payload: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let mut v = Vec::new();
    for wt in [0u8, 1, 2, 5, 3] {
        if wt == natural {
            continue;
        }
        let mut o = tag(num, wt);
        match wt {
            0 => o.push(1),
            1 => o.extend([0u8; 8]),
            5 => o.extend([0u8; 4]),
            2 => {
                put_varint(&mut o, payload.len() as u64);
                o.extend_from_slice(payload);
            }
            _ => o.extend(tag(num, 4)),
        }
        v.push((wt, o));
    }
    v
}

const UNKNOWN_NUMS: &[u64] = &[
    1023,
    1024,
    1025,
    1031,
    2047,
    2048,
    3072,
    19_000,
    19_999,
    (1 << 29) - 1,
    1 << 29,
    (1 << 31) - 1,
    1 << 31,
];

type Wrap<'a> = &'a dyn Fn(Vec<u8>) -> Vec<u8>;

#[allow(clippy::too_many_lines)]
fn walk(
    info: &'static MessageInfo,
    b: &[u8],
    depth: usize,
    path: &str,
    wrap: Wrap<'_>,
    out: &mut Vec<Mutation>,
) {
    macro_rules! push {
        ($class:expr, $p:expr, $bytes:expr $(,)?) => {
            out.push(Mutation {
                class: $class,
                path: $p,
                bytes: wrap($bytes),
            })
        };
    }
    let Some(fs) = parse(b) else {
        return;
    };
    let max_known = info
        .fields
        .iter()
        .map(|f| u64::from(f.number))
        .max()
        .unwrap_or(0);
    let known = |n: u64| u32::try_from(n).ok().and_then(|n| info.field(n));
    let ends: Vec<usize> = if fs.is_empty() {
        vec![0]
    } else {
        vec![0, fs.len()]
    };

    // Unknown fields, critical and non-critical, every wire type.
    let mut nums: Vec<u64> = vec![max_known + 1];
    nums.extend(UNKNOWN_NUMS.iter().copied());
    // Keys past u32::MAX: gogoproto's int32 truncation makes them field
    // `max_known + 1` (unknown) or an existing field (alias).
    nums.push((1u64 << 32) + max_known + 1);
    if let Some(f) = info.fields.first() {
        nums.push((1u64 << 32) + u64::from(f.number));
    }
    for &n in &nums {
        if known(n).is_some() {
            continue;
        }
        for (k, raw) in [
            tag(n, 0).into_iter().chain([1]).collect::<Vec<u8>>(),
            tag(n, 1).into_iter().chain([0; 8]).collect(),
            tag(n, 5).into_iter().chain([0; 4]).collect(),
            len_field(n, b"\x08\x01"),
        ]
        .into_iter()
        .enumerate()
        {
            for &at in &ends {
                if k > 0 && at == 0 && n != max_known + 1 {
                    continue;
                }
                push!(
                    "unknown",
                    format!("{path}+{n}/wt{k}@{at}"),
                    with_inserted(&fs, at, &raw),
                );
            }
        }
    }
    // An alias key (> u32::MAX) of a known field carrying its natural payload.
    for f in info.fields.iter().take(3) {
        let n = (1u64 << 32) + u64::from(f.number);
        let raw = match fs.iter().find(|x| x.num == u64::from(f.number)) {
            Some(x) => {
                let mut o = tag(n, x.wt);
                if x.wt == 2 {
                    put_varint(&mut o, x.val.len() as u64);
                }
                o.extend_from_slice(&x.val);
                o
            }
            None => continue,
        };
        push!(
            "unknown_alias",
            format!("{path}+{n}"),
            with_inserted(&fs, fs.len(), &raw)
        );
    }

    // Groups (wire types 3 / 4) at the start, middle and end.
    let gnum = max_known + 1;
    let mut groups: Vec<(&'static str, Vec<u8>)> = vec![
        ("group", [tag(gnum, 3), tag(gnum, 4)].concat()),
        (
            "group",
            [tag(gnum, 3), vec![0x08, 0x01], tag(gnum, 4)].concat(),
        ),
        (
            "group",
            [
                tag(gnum, 3),
                tag(gnum + 1, 3),
                tag(gnum + 1, 4),
                tag(gnum, 4),
            ]
            .concat(),
        ),
        ("group", [tag(1025, 3), tag(1025, 4)].concat()),
        ("group_mismatch", [tag(gnum, 3), tag(gnum + 1, 4)].concat()),
        (
            "group_mismatch",
            [tag(gnum, 3), vec![0x08, 0x01], tag(1, 4)].concat(),
        ),
        ("group_lone_end", tag(gnum, 4)),
        ("group_open", tag(gnum, 3)),
    ];
    for f in info.fields.iter().take(4) {
        let n = u64::from(f.number);
        groups.push(("group_known", [tag(n, 3), tag(n, 4)].concat()));
    }
    let mut positions = ends.clone();
    if fs.len() >= 2 {
        positions.push(fs.len() / 2);
    }
    for (class, raw) in &groups {
        for &at in &positions {
            push!(class, format!("{path}@{at}"), with_inserted(&fs, at, raw));
        }
    }
    // Field number 0 and wire types 6 / 7.
    for raw in [vec![0x00, 0x00], vec![0x02, 0x00], tag(1, 6), tag(1, 7)] {
        let class = if raw[0] >> 3 == 0 { "field0" } else { "wt67" };
        push!(class, path.to_owned(), with_inserted(&fs, fs.len(), &raw));
    }

    // Reordering.
    if fs.len() >= 2 {
        let mut rev = fs.clone();
        rev.reverse();
        push!(
            "reorder_rev",
            path.to_owned(),
            with_replaced(&rev, usize::MAX, None)
        );
        let mut rot = fs.clone();
        rot.rotate_left(1);
        push!(
            "reorder_rot",
            path.to_owned(),
            with_replaced(&rot, usize::MAX, None)
        );
    }

    // Absent fields with their defaults; oneof pairs.
    for f in info.fields {
        let n = u64::from(f.number);
        if fs.iter().any(|x| x.num == n) {
            continue;
        }
        for raw in default_encodings(f) {
            push!(
                "default_explicit",
                format!("{path}.{n}"),
                with_inserted(&fs, fs.len(), &raw),
            );
        }
    }
    for f in info.fields.iter().filter(|f| f.oneof >= 0) {
        let n = u64::from(f.number);
        let Some(i) = fs.iter().position(|x| x.num == n) else {
            continue;
        };
        for g in info
            .fields
            .iter()
            .filter(|g| g.oneof == f.oneof && g.number != f.number)
        {
            let gn = u64::from(g.number);
            let raw = default_encodings(g).into_iter().next().unwrap_or_default();
            push!(
                "oneof_two",
                format!("{path}.{n}|{gn}"),
                with_inserted(&fs, i + 1, &raw),
            );
            push!(
                "oneof_two_first",
                format!("{path}.{gn}|{n}"),
                with_inserted(&fs, i, &raw),
            );
        }
    }

    // Repeated scalars: packing.
    for f in info
        .fields
        .iter()
        .filter(|f| f.repeated && is_numeric(f.ty))
    {
        let n = u64::from(f.number);
        let nat = natural_wt(f);
        let mut elems: Vec<Vec<u8>> = Vec::new();
        for x in fs.iter().filter(|x| x.num == n) {
            if x.wt == nat {
                elems.push(x.val.clone());
            } else if x.wt == 2 {
                let mut r = x.val.as_slice();
                while !r.is_empty() {
                    let w = match nat {
                        0 => r.iter().position(|&c| c < 0x80).map_or(r.len(), |p| p + 1),
                        5 => 4.min(r.len()),
                        _ => 8.min(r.len()),
                    };
                    elems.push(r[..w].to_vec());
                    r = &r[w..];
                }
            }
        }
        if elems.is_empty() {
            continue;
        }
        let first = fs.iter().position(|x| x.num == n).unwrap_or(fs.len());
        let rest: Vec<Field> = fs.iter().filter(|x| x.num != n).cloned().collect();
        let at = first.min(rest.len());
        let unpacked: Vec<u8> = elems
            .iter()
            .flat_map(|e| [tag(n, nat), e.clone()].concat())
            .collect();
        let packed = len_field(n, &elems.concat());
        let half = elems.len().div_ceil(2);
        let mixed = [
            len_field(n, &elems[..half].concat()),
            elems[half..]
                .iter()
                .flat_map(|e| [tag(n, nat), e.clone()].concat())
                .collect(),
            len_field(n, &[]),
        ]
        .concat();
        let empty_run = [len_field(n, &[]), packed.clone()].concat();
        let mut runs = vec![
            ("unpacked", unpacked),
            ("packed", packed),
            ("packed_mixed", mixed),
            ("packed_empty", empty_run),
        ];
        if nat == 0 {
            // Varints inside a packed run: the tx decoder's unknown-field
            // check does not parse them, gogoproto reads 10 bytes with any
            // last byte, prost and protobuf-go reject a last byte above 1.
            let vals: Vec<u64> = elems.iter().map(|e| read_varint(e).unwrap_or(0)).collect();
            let long: Vec<u8> = vals.iter().flat_map(|&v| varint_long10(v, 0x3f)).collect();
            let nonmin: Vec<u8> = vals.iter().flat_map(|&v| varint_nonmin(v, 2)).collect();
            runs.push(("packed_long10", len_field(n, &long)));
            runs.push(("packed_nonmin", len_field(n, &nonmin)));
            runs.push((
                "unpacked_long10",
                vals.iter()
                    .flat_map(|&v| [tag(n, 0), varint_long10(v, 1)].concat())
                    .collect(),
            ));
        }
        for (class, raw) in runs {
            push!(class, format!("{path}.{n}"), with_inserted(&rest, at, &raw));
        }
        if !rest.is_empty() && elems.len() >= 2 {
            // Elements spread around the other fields.
            let mut o = Vec::new();
            let mut e = elems.iter();
            for r in &rest {
                if let Some(x) = e.next() {
                    o.extend([tag(n, nat), x.clone()].concat());
                }
                o.extend(field_bytes(r));
            }
            for x in e {
                o.extend([tag(n, nat), x.clone()].concat());
            }
            push!("repeated_interleaved", format!("{path}.{n}"), o);
        }
    }

    // Per present field (the first two occurrences of a repeated one).
    let mut seen: std::collections::BTreeMap<u64, usize> = std::collections::BTreeMap::new();
    for (i, x) in fs.iter().enumerate() {
        let n = x.num;
        let p = format!("{path}.{n}");
        let Some(f) = known(n) else {
            continue;
        };
        let k = seen.entry(n).or_default();
        *k += 1;
        if *k > 2 || out.len() >= MAX_MUTATIONS {
            continue;
        }
        let nat = natural_wt(f);
        // Non-minimal and 10-byte varints in the key, value, length.
        let mut nm_tag = varint_nonmin((n << 3) | u64::from(x.wt), 1);
        let mut l10_tag = varint_long10((n << 3) | u64::from(x.wt), 0x3f);
        let tail: Vec<u8> = if x.wt == 2 {
            [varint(x.val.len() as u64), x.val.clone()].concat()
        } else {
            x.val.clone()
        };
        nm_tag.extend_from_slice(&tail);
        l10_tag.extend_from_slice(&tail);
        push!(
            "nonmin_tag",
            p.clone(),
            with_replaced(&fs, i, Some(&nm_tag))
        );
        push!(
            "long10_tag",
            p.clone(),
            with_replaced(&fs, i, Some(&l10_tag))
        );
        if x.wt == 0 {
            let v = read_varint(&x.val).unwrap_or(0);
            for (class, enc) in [
                ("nonmin_val", varint_nonmin(v, 1)),
                ("nonmin_val", varint_nonmin(v, 9 - varint(v).len().min(9))),
                ("long10_val", varint_long10(v, 1)),
                ("long10_val", varint_long10(v, 0x3f)),
            ] {
                if enc.len() > 10 {
                    continue;
                }
                let raw = [tag(n, 0), enc].concat();
                push!(class, p.clone(), with_replaced(&fs, i, Some(&raw)));
            }
        }
        if x.wt == 2 {
            for (class, enc) in [
                ("nonmin_len", varint_nonmin(x.val.len() as u64, 1)),
                ("long10_len", varint_long10(x.val.len() as u64, 0x3f)),
            ] {
                let raw = [tag(n, 2), enc, x.val.clone()].concat();
                push!(class, p.clone(), with_replaced(&fs, i, Some(&raw)));
            }
        }
        // Wrong wire types.
        let payload: Vec<u8> = x.val.clone();
        for (wt, raw) in wrong_wire(
            n,
            if f.ty == ty::MESSAGE || !is_numeric(f.ty) {
                2
            } else {
                nat
            },
            &payload,
        ) {
            if f.repeated && is_numeric(f.ty) && wt == 2 {
                continue; // packed: valid
            }
            push!(
                "wrong_wt",
                format!("{p}/wt{wt}"),
                with_replaced(&fs, i, Some(&raw))
            );
        }
        // Removal.
        push!("drop", p.clone(), with_replaced(&fs, i, None));

        if f.ty == ty::MESSAGE {
            if x.wt != 2 {
                continue;
            }
            let child_info = wire_table::message(f.type_name);
            // Empty in place.
            push!(
                "zero_len",
                p.clone(),
                with_replaced(&fs, i, Some(&len_field(n, &[])))
            );
            if !f.repeated {
                // Repeated singular message: gogoproto merges (stdtime /
                // stdduration overwrite, a oneof member replaces).
                let own = field_bytes(x);
                push!("dup_same", p.clone(), with_inserted(&fs, i + 1, &own));
                push!(
                    "msg_empty_last",
                    p.clone(),
                    with_inserted(&fs, i + 1, &len_field(n, &[])),
                );
                push!(
                    "msg_empty_first",
                    p.clone(),
                    with_inserted(&fs, i, &len_field(n, &[])),
                );
                if let Some(sub) = parse(&x.val) {
                    for k in 1..sub.len() {
                        let a: Vec<u8> = sub[..k].iter().flat_map(field_bytes).collect();
                        let bb: Vec<u8> = sub[k..].iter().flat_map(field_bytes).collect();
                        let raw = [len_field(n, &a), len_field(n, &bb)].concat();
                        push!(
                            "msg_split",
                            format!("{p}/{k}"),
                            with_replaced(&fs, i, Some(&raw))
                        );
                        push!(
                            "msg_part_last",
                            format!("{p}/{k}"),
                            with_inserted(&fs, i + 1, &len_field(n, &bb)),
                        );
                    }
                    // A copy with one scalar changed, after the original.
                    if let Some(ci) = child_info {
                        for (k, y) in sub.iter().enumerate() {
                            let Some(cf) = u32::try_from(y.num).ok().and_then(|m| ci.field(m))
                            else {
                                continue;
                            };
                            if cf.ty == ty::MESSAGE || cf.repeated {
                                continue;
                            }
                            if let Some(alt) = scalar_alts(cf, &y.val).into_iter().next() {
                                let mut sub2 = sub.clone();
                                sub2[k].val = alt;
                                if cf.ty == ty::MESSAGE || !is_numeric(cf.ty) {
                                    sub2[k].wt = 2;
                                }
                                let v: Vec<u8> = sub2.iter().flat_map(field_bytes).collect();
                                push!(
                                    "msg_alt_last",
                                    format!("{p}.{}", y.num),
                                    with_inserted(&fs, i + 1, &len_field(n, &v)),
                                );
                            }
                        }
                    }
                }
            }
            // Recurse.
            if depth < MAX_DEPTH
                && let Some(ci) = child_info
            {
                let fs2 = fs.clone();
                let child_wrap =
                    move |v: Vec<u8>| wrap(with_replaced(&fs2, i, Some(&len_field(n, &v))));
                if ci.name == "google.protobuf.Any" {
                    walk_any(&x.val, depth + 1, &p, &child_wrap, out);
                } else {
                    walk(ci, &x.val, depth + 1, &p, &child_wrap, out);
                }
            }
            continue;
        }
        // Scalars, strings, bytes, customtypes.
        if !f.repeated {
            let own = field_bytes(x);
            push!("dup_same", p.clone(), with_inserted(&fs, i + 1, &own));
            for alt in scalar_alts(f, &x.val) {
                let raw = field_bytes(&Field {
                    num: n,
                    wt: x.wt,
                    val: alt,
                });
                push!("dup_last", p.clone(), with_inserted(&fs, i + 1, &raw));
                push!("dup_first", p.clone(), with_inserted(&fs, i, &raw));
            }
        }
    }
}

/// An `Any` node: its own edge cases, then the value as its type.
fn walk_any(b: &[u8], depth: usize, path: &str, wrap: Wrap<'_>, out: &mut Vec<Mutation>) {
    let Some(fs) = parse(b) else {
        return;
    };
    let url = fs
        .iter()
        .rev()
        .find(|f| f.num == 1 && f.wt == 2)
        .map(|f| f.val.clone())
        .unwrap_or_default();
    let value = fs
        .iter()
        .rev()
        .find(|f| f.num == 2 && f.wt == 2)
        .map(|f| f.val.clone())
        .unwrap_or_default();
    let any = |u: &[u8], v: &[u8]| -> Vec<u8> {
        let mut o = Vec::new();
        if !u.is_empty() {
            o.extend(len_field(1, u));
        }
        if !v.is_empty() {
            o.extend(len_field(2, v));
        }
        o
    };
    macro_rules! push {
        ($class:expr, $bytes:expr $(,)?) => {
            out.push(Mutation {
                class: $class,
                path: path.to_owned(),
                bytes: wrap($bytes),
            })
        };
    }
    let bare = url.strip_prefix(b"/").unwrap_or(&url).to_vec();
    for (class, u) in [
        ("any_url_noslash", bare.clone()),
        (
            "any_url_host",
            [b"type.googleapis.com/".as_slice(), &bare].concat(),
        ),
        ("any_url_double", [b"//".as_slice(), &bare].concat()),
        ("any_url_trailing", [url.as_slice(), b" "].concat()),
        ("any_url_empty", Vec::new()),
    ] {
        push!(class, any(&u, &value));
    }
    push!("any_value_empty", any(&url, &[]));
    push!(
        "any_value_trailing",
        any(&url, &[value.as_slice(), &[0x00]].concat())
    );
    push!(
        "any_value_trailing",
        any(&url, &[value.as_slice(), &tag(2047, 0), &[1]].concat()),
    );
    push!(
        "any_value_trailing",
        any(&url, &[value.as_slice(), &tag(20, 0), &[1]].concat()),
    );
    if !value.is_empty() {
        push!("any_value_truncated", any(&url, &value[..value.len() - 1]));
    }
    // Type URL and value in separate occurrences of the Any field are a
    // parent-level edit; here: two type URLs / two values.
    push!(
        "any_url_twice",
        [
            len_field(1, b"/x"),
            len_field(1, &url),
            len_field(2, &value)
        ]
        .concat(),
    );
    push!(
        "any_value_twice",
        [
            len_field(1, &url),
            len_field(2, b"\x0a\x01x"),
            len_field(2, &value)
        ]
        .concat(),
    );
    // The value as its registered type.
    if let Some(info) = type_name(&url) {
        let url2 = url.clone();
        let w = move |v: Vec<u8>| wrap(any(&url2, &v));
        let tn = info.name;
        walk(info, &value, depth, &format!("{path}@{tn}"), &w, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_forms() {
        assert_eq!(varint_nonmin(1, 1), vec![0x81, 0x00]);
        assert_eq!(varint_nonmin(0, 2), vec![0x80, 0x80, 0x00]);
        let l = varint_long10(5, 1);
        assert_eq!(l.len(), 10);
        assert_eq!(l[9], 2);
        assert_eq!(read_varint(&l), Some(5));
    }

    #[test]
    fn enumerates_every_class_for_a_send() {
        use celestia_proto::cosmos::bank::v1beta1::MsgSend;
        use celestia_proto::cosmos::base::v1beta1::Coin;
        use prost::Message as _;
        let m = MsgSend {
            from_address: "a".into(),
            to_address: "b".into(),
            amount: vec![Coin {
                denom: "utia".into(),
                amount: "5".into(),
            }],
        }
        .encode_to_vec();
        let ms = enumerate("cosmos.bank.v1beta1.MsgSend", &m);
        let classes: std::collections::BTreeSet<_> = ms.iter().map(|m| m.class).collect();
        for c in [
            "unknown",
            "group",
            "group_mismatch",
            "dup_last",
            "dup_first",
            "nonmin_tag",
            "long10_len",
            "wrong_wt",
            "reorder_rev",
        ] {
            assert!(classes.contains(c), "{c} missing from {classes:?}");
        }
        // Nested: the coin's amount is repeated too.
        assert!(
            ms.iter()
                .any(|m| m.class == "dup_last" && m.path.ends_with(".3.2"))
        );
    }

    #[test]
    fn singular_messages_and_stdtime_are_split() {
        use celestia_proto::cosmos::authz::v1beta1::{Grant, MsgGrant};
        use prost::Message as _;
        let m = MsgGrant {
            granter: "a".into(),
            grantee: "b".into(),
            grant: Some(Grant {
                authorization: Some(celestia_proto::Any {
                    type_url: "/cosmos.authz.v1beta1.GenericAuthorization".into(),
                    value: vec![0x0a, 0x01, b'x'],
                }),
                expiration: Some(pbjson_types::Timestamp {
                    seconds: 9,
                    nanos: 5,
                }),
            }),
        }
        .encode_to_vec();
        let ms = enumerate("cosmos.authz.v1beta1.MsgGrant", &m);
        // The expiration's seconds and nanos in two occurrences.
        assert!(
            ms.iter()
                .any(|m| m.class == "msg_split" && m.path.ends_with(".3.2/1"))
        );
        // Inside the authorization `Any`, resolved by its type URL.
        assert!(
            ms.iter()
                .any(|m| m.class == "dup_last" && m.path.contains("GenericAuthorization.1"))
        );
        assert!(ms.iter().any(|m| m.class == "any_url_noslash"));
    }
}
