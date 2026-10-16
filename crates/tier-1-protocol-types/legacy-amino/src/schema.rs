//! The Go type graph go-amino reflects over (`schema.json`).

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value as J;

use crate::V;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Bool,
    Int32,
    Int64,
    Uint,
    Uint8,
    Uint32,
    Uint64,
    String,
    Slice,
    Array,
    Ptr,
    Struct,
    Interface,
}

/// Go types whose (de)serialization overrides go-amino's reflection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Special {
    None,
    Time,
    Int,
    Dec,
    Any,
    Coins,
    HexAddress,
    /// An amino key type (`MarshalAmino` = its `Key` bytes): the required
    /// key length and whether a wrong length is a pubkey error.
    Key(usize, bool),
    Multisig,
    /// A type with a JSON marshaler this port does not implement.
    Unsupported,
}

#[derive(Clone, Debug)]
pub(crate) struct Field {
    pub json: String,
    pub omitempty: bool,
    pub unpacked: bool,
    pub ty: usize,
    /// The interface `UnpackInterfaces` unpacks this `Any` field into.
    pub any_iface: Option<usize>,
}

#[derive(Clone, Debug)]
pub(crate) struct Reg {
    pub name: String,
    pub prefix: [u8; 4],
    pub disamb: [u8; 3],
    pub ptr_preferred: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct TypeDef {
    /// Go `reflect.Type.String()`.
    pub go: String,
    pub kind: Kind,
    pub elem: usize,
    pub len: usize,
    pub fields: Vec<Field>,
    pub special: Special,
    pub reg: Option<Reg>,
    /// Interface implementers by prefix.
    pub implementers: HashMap<[u8; 4], Vec<usize>>,
    pub always_disamb: bool,
}

/// The schema.
#[derive(Debug)]
pub struct Schema {
    pub(crate) types: Vec<TypeDef>,
    pub(crate) root: usize,
    pub(crate) by_name: HashMap<String, usize>,
    pub(crate) by_disfix: HashMap<[u8; 7], usize>,
    pub(crate) multisig_repr: usize,
}

const SCHEMA: &str = include_str!("schema.json");
const SCHEMA_V9: &str = include_str!("schema_v9.json");

fn kind(s: &str) -> Kind {
    match s {
        "bool" => Kind::Bool,
        "int32" => Kind::Int32,
        "int64" => Kind::Int64,
        "uint" => Kind::Uint,
        "uint8" => Kind::Uint8,
        "uint32" => Kind::Uint32,
        "uint64" => Kind::Uint64,
        "string" => Kind::String,
        "slice" => Kind::Slice,
        "array" => Kind::Array,
        "ptr" => Kind::Ptr,
        "struct" => Kind::Struct,
        // The schema check test pins the set of kinds.
        _ => Kind::Interface,
    }
}

fn special(id: &str, t: &J) -> Special {
    match id {
        "time.Time" => Special::Time,
        "cosmossdk.io/math.Int" => Special::Int,
        "cosmossdk.io/math.LegacyDec" => Special::Dec,
        "github.com/cosmos/cosmos-sdk/codec/types.Any" => Special::Any,
        "github.com/cosmos/cosmos-sdk/types.Coins" => Special::Coins,
        "github.com/bcp-innovations/hyperlane-cosmos/util.HexAddress" => Special::HexAddress,
        "github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1.PubKey" => Special::Key(33, true),
        "github.com/cosmos/cosmos-sdk/crypto/keys/secp256k1.PrivKey" => Special::Key(32, false),
        "github.com/cosmos/cosmos-sdk/crypto/keys/ed25519.PubKey" => Special::Key(32, true),
        "github.com/cosmos/cosmos-sdk/crypto/keys/ed25519.PrivKey" => Special::Key(64, false),
        "github.com/cosmos/cosmos-sdk/crypto/keys/multisig.LegacyAminoPubKey" => Special::Multisig,
        _ => {
            let custom = ["json_marshal", "amino_marshal", "amino_json_marshal"]
                .iter()
                .any(|k| t.get(k).is_some());
            if custom {
                Special::Unsupported
            } else {
                Special::None
            }
        }
    }
}

fn hex_arr<const N: usize>(s: &str) -> [u8; N] {
    let mut out = [0u8; N];
    for (i, o) in out.iter_mut().enumerate() {
        *o = s
            .get(2 * i..2 * i + 2)
            .and_then(|h| u8::from_str_radix(h, 16).ok())
            .unwrap_or(0);
    }
    out
}

impl Schema {
    /// The embedded celestia-appd v10.4.0 schema.
    #[must_use]
    pub fn get() -> &'static Self {
        static S: OnceLock<Schema> = OnceLock::new();
        S.get_or_init(|| Self::parse(SCHEMA))
    }

    /// The embedded celestia-appd v9.0.8 schema (`schema_v9.json`:
    /// hyperlane-cosmos v1.1.0's amino names, no fibre types).
    #[must_use]
    pub fn v9() -> &'static Self {
        static S: OnceLock<Schema> = OnceLock::new();
        S.get_or_init(|| Self::parse(SCHEMA_V9))
    }

    /// The schema of the celestia-app release that runs `app_version`
    /// (ADR 0004: below 10 is v9.0.8; 0, unset, and 10 are v10.4.0).
    #[must_use]
    pub fn for_app_version(app_version: u64) -> &'static Self {
        if app_version != 0 && app_version < 10 {
            Self::v9()
        } else {
            Self::get()
        }
    }

    fn parse(src: &str) -> Self {
        let j: J = serde_json::from_str(src).unwrap_or(J::Null);
        let types_j = j["types"].as_object().cloned().unwrap_or_default();
        let ids: Vec<&String> = types_j.keys().collect();
        let idx: HashMap<&str, usize> = ids
            .iter()
            .enumerate()
            .map(|(i, k)| (k.as_str(), i))
            .collect();
        let at = |s: &J| s.as_str().and_then(|s| idx.get(s).copied()).unwrap_or(0);
        let any_fields = j["any_fields"].as_object().cloned().unwrap_or_default();
        let mut types = Vec::with_capacity(ids.len());
        let mut by_name = HashMap::new();
        let mut by_disfix = HashMap::new();
        for (i, id) in ids.iter().enumerate() {
            let t = &types_j[id.as_str()];
            let fields = t["fields"]
                .as_array()
                .map(|fs| {
                    fs.iter()
                        .map(|f| {
                            let name = f["name"].as_str().unwrap_or_default().to_owned();
                            Field {
                                any_iface: any_fields.get(&format!("{id}.{name}")).map(at),
                                json: f["json"].as_str().unwrap_or_default().to_owned(),
                                omitempty: f["omitempty"].as_bool().unwrap_or(false),
                                unpacked: f["unpacked"].as_bool().unwrap_or(false),
                                ty: at(&f["type"]),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            let reg = t.get("registered").map(|r| Reg {
                name: r["name"].as_str().unwrap_or_default().to_owned(),
                prefix: hex_arr(r["prefix"].as_str().unwrap_or_default()),
                disamb: hex_arr(r["disamb"].as_str().unwrap_or_default()),
                ptr_preferred: r["ptr_preferred"].as_bool().unwrap_or(false),
            });
            if let Some(r) = &reg {
                by_name.insert(r.name.clone(), i);
                let mut df = [0u8; 7];
                df[..3].copy_from_slice(&r.disamb);
                df[3..].copy_from_slice(&r.prefix);
                by_disfix.insert(df, i);
            }
            let mut implementers = HashMap::new();
            if let Some(m) = t["iface"]["implementers"].as_object() {
                for (p, list) in m {
                    let v: Vec<usize> = list
                        .as_array()
                        .map(|l| l.iter().map(&at).collect())
                        .unwrap_or_default();
                    implementers.insert(hex_arr(p), v);
                }
            }
            types.push(TypeDef {
                go: t["go"].as_str().unwrap_or_default().to_owned(),
                kind: kind(t["kind"].as_str().unwrap_or_default()),
                elem: at(&t["elem"]),
                len: usize::try_from(t["len"].as_u64().unwrap_or(0)).unwrap_or(0),
                fields,
                special: special(id, t),
                reg,
                implementers,
                always_disamb: t["iface"]["always_disamb"].as_bool().unwrap_or(false),
            });
        }
        Self {
            root: at(&j["root"]),
            multisig_repr: idx
                .get("github.com/cosmos/cosmos-sdk/crypto/keys/multisig.tmMultisig")
                .copied()
                .unwrap_or(0),
            types,
            by_name,
            by_disfix,
        }
    }

    pub(crate) fn t(&self, i: usize) -> &TypeDef {
        &self.types[i]
    }

    /// The type behind any pointers.
    pub(crate) fn deref(&self, mut i: usize) -> usize {
        while self.types[i].kind == Kind::Ptr {
            i = self.types[i].elem;
        }
        i
    }

    /// Whether `[]elem` / `[N]elem` is a byte slice / array.
    pub(crate) fn is_bytes(&self, i: usize) -> bool {
        matches!(self.types[i].kind, Kind::Slice | Kind::Array)
            && self.types[self.types[i].elem].kind == Kind::Uint8
    }

    /// The Go zero value of type `i` (`reflect.Zero`).
    pub(crate) fn zero(&self, i: usize) -> V {
        let t = &self.types[i];
        match t.special {
            Special::Time => return V::Time(crate::time::ZERO_SECS, 0),
            Special::Int => return V::BigInt(None),
            Special::Dec => return V::Dec(None),
            Special::Any => return V::Any(crate::AnyV::Zero),
            _ => {}
        }
        match t.kind {
            Kind::Bool => V::Bool(false),
            Kind::Int32 | Kind::Int64 => V::Int(0),
            Kind::Uint | Kind::Uint8 | Kind::Uint32 | Kind::Uint64 => V::Uint(0),
            Kind::String => V::Str(Vec::new()),
            Kind::Slice | Kind::Ptr | Kind::Interface => V::Nil,
            Kind::Array => {
                if self.is_bytes(i) {
                    V::Bytes(vec![0; t.len])
                } else {
                    V::List((0..t.len).map(|_| self.zero(t.elem)).collect())
                }
            }
            Kind::Struct => V::Struct(t.fields.iter().map(|f| self.zero(f.ty)).collect()),
        }
    }

    /// go-amino `defaultValue`: the zero value, except that a `time.Time`
    /// (or a pointer chain to one) is 1970-01-01.
    pub(crate) fn default_value(&self, i: usize) -> V {
        let t = &self.types[i];
        if t.kind == Kind::Ptr {
            if self.types[self.deref(i)].special == Special::Time {
                let mut v = V::Time(0, 0);
                let mut j = i;
                let mut depth = 0;
                while self.types[j].kind == Kind::Ptr {
                    depth += 1;
                    j = self.types[j].elem;
                }
                for _ in 0..depth {
                    v = V::Ptr(Box::new(v));
                }
                return v;
            }
            return V::Nil;
        }
        if t.special == Special::Time {
            return V::Time(0, 0);
        }
        self.zero(i)
    }

    /// `reflect.DeepEqual(v, reflect.Zero(type i))`.
    pub(crate) fn is_zero(&self, i: usize, v: &V) -> bool {
        let t = &self.types[i];
        match v {
            V::Nil => true,
            V::Bool(b) => !b,
            V::Int(x) => *x == 0,
            V::Uint(x) => *x == 0,
            V::Str(s) => s.is_empty(),
            V::Bytes(b) => t.kind == Kind::Array && b.iter().all(|x| *x == 0),
            V::List(l) => t.kind == Kind::Array && l.iter().all(|e| self.is_zero(t.elem, e)),
            V::Struct(fs) => t.fields.iter().zip(fs).all(|(f, v)| self.is_zero(f.ty, v)),
            V::Ptr(_) | V::Iface(..) => false,
            V::Time(s, n) => *s == crate::time::ZERO_SECS && *n == 0,
            V::BigInt(b) | V::Dec(b) => b.is_none(),
            V::Any(a) => *a == crate::AnyV::Zero,
        }
    }

    /// The `%v` of `reflect.TypeOf(iface)` for a pointer to interface `i`.
    pub(crate) fn ptr_go(&self, i: usize) -> String {
        format!("*{}", self.types[i].go)
    }
}
