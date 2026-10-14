//! `cargo xtask gen-proto` — regenerate committed prost bindings from the
//! vendored `.proto` sources under `proto/`.
//!
//! Uses `protox` (a pure-Rust protobuf compiler) so no `protoc` binary is
//! required. Generated files are committed; regenerate only after bumping
//! the vendored proto sources.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// One generation target: a set of proto roots compiled into one crate.
struct Target {
    /// Directories searched for imports (relative to repo root).
    includes: &'static [&'static str],
    /// Directories whose `.proto` files are compiled (relative to repo root).
    sources: &'static [&'static str],
    /// Output directory for generated Rust (relative to repo root).
    out: &'static str,
    /// `(proto path, rust path)` pairs resolved to other crates.
    extern_paths: &'static [(&'static str, &'static str)],
    /// Emit `registry.rs` mapping `Any` type URLs to JSON (de)serializers.
    any_registry: bool,
}

const TARGETS: &[Target] = &[
    Target {
        includes: &["proto/celestia-core", "proto/third_party"],
        sources: &["proto/celestia-core"],
        out: "crates/tier-1-protocol-types/celestia-core-proto/src/gen",
        extern_paths: &[(".google.protobuf", "::pbjson_types")],
        any_registry: false,
    },
    Target {
        includes: &[
            "proto/cosmos-sdk",
            "proto/ibc-go",
            "proto/celestia-app",
            "proto/hyperlane-cosmos",
            "proto/packet-forward-middleware",
            "proto/celestia-core",
            "proto/third_party",
        ],
        sources: &[
            "proto/cosmos-sdk",
            "proto/ibc-go",
            "proto/celestia-app",
            "proto/hyperlane-cosmos",
            "proto/packet-forward-middleware",
        ],
        out: "crates/tier-1-protocol-types/celestia-proto/src/gen",
        extern_paths: &[
            (".tendermint", "::celestia_core_proto::tendermint"),
            (".cosmos.ics23.v1", "::ics23"),
            (".google.protobuf.Any", "crate::any::Any"),
            (".google.protobuf", "::pbjson_types"),
        ],
        any_registry: true,
    },
];

pub fn run(dry_run: bool) -> Result<()> {
    let root = repo_root()?;
    for target in TARGETS {
        let mut files = Vec::new();
        for src in target.sources {
            collect_protos(&root.join(src), &mut files)?;
        }
        files.sort();
        let out = root.join(target.out);
        println!("gen-proto: {} files -> {}", files.len(), out.display());
        if dry_run {
            continue;
        }
        let includes: Vec<PathBuf> = target.includes.iter().map(|i| root.join(i)).collect();
        let mut compiler =
            protox::Compiler::new(&includes).map_err(|e| anyhow::anyhow!("protox: {e:?}"))?;
        compiler.include_imports(true);
        compiler
            .open_files(&files)
            .map_err(|e| anyhow::anyhow!("protox: {e:?}"))?;
        let pool = compiler.descriptor_pool();
        let fds = compiler.file_descriptor_set();
        let fds_bytes = prost::Message::encode_to_vec(&fds);
        if out.exists() {
            std::fs::remove_dir_all(&out)?;
        }
        std::fs::create_dir_all(&out)?;
        let mut config = prost_build::Config::new();
        config.out_dir(&out);
        config.btree_map(["."]);
        config.include_file("mod.rs");
        for (proto, rust) in target.extern_paths {
            config.extern_path(*proto, *rust);
        }
        config.enable_type_names();
        config.compile_well_known_types();
        let packages: std::collections::BTreeSet<String> = fds
            .file
            .iter()
            .filter(|f| {
                target
                    .sources
                    .iter()
                    .any(|src| root.join(src).join(f.name()).exists())
            })
            .map(|f| f.package().to_owned())
            .collect();
        let messages: Vec<(String, String)> = fds
            .file
            .iter()
            .filter(|f| packages.contains(f.package()))
            .flat_map(|f| {
                f.message_type
                    .iter()
                    .map(move |m| (f.package().to_owned(), m.name().to_owned()))
            })
            .collect();
        config
            .compile_fds(fds)
            .context("prost-build code generation")?;

        let mut pbjson = pbjson_build::Builder::new();
        pbjson
            .register_descriptors(&fds_bytes)
            .context("pbjson descriptors")?
            .out_dir(&out)
            .preserve_proto_field_names()
            .btree_map(["."])
            .emit_fields();
        for (proto, rust) in target.extern_paths {
            pbjson.extern_path(*proto, *rust);
        }
        let selectors: Vec<String> = packages.iter().map(|p| format!(".{p}")).collect();
        pbjson.build(&selectors).context("pbjson code generation")?;
        // Attach the serde impls to each package module.
        for pkg in &packages {
            let rs = out.join(format!("{pkg}.rs"));
            let serde = format!("{pkg}.serde.rs");
            if rs.exists() && out.join(&serde).exists() {
                let mut body = std::fs::read_to_string(&rs)?;
                {
                    use std::fmt::Write as _;
                    writeln!(body, "include!(\"{serde}\");")?;
                }
                std::fs::write(&rs, body)?;
            }
        }
        if target.any_registry {
            write_registry(&out, &messages)?;
            write_signers(&out, &pool, &packages)?;
        }
    }
    Ok(())
}

/// Writes `registry.rs`: JSON <-> protobuf-bytes conversion keyed by
/// `Any` type URL, for every top-level message of the target's packages.
fn write_registry(out: &Path, messages: &[(String, String)]) -> Result<()> {
    use std::fmt::Write as _;
    let mut decode = String::new();
    let mut encode = String::new();
    for (pkg, name) in messages {
        use heck::ToUpperCamelCase as _;
        let module: Vec<String> = pkg
            .split('.')
            .map(heck::ToSnakeCase::to_snake_case)
            .collect();
        let path = format!(
            "crate::{}::{}",
            module.join("::"),
            name.to_upper_camel_case()
        );
        let url = format!("/{pkg}.{name}");
        writeln!(decode, "        {url:?} => json_to_bytes::<{path}>(value),")?;
        writeln!(encode, "        {url:?} => bytes_to_json::<{path}>(bytes),")?;
    }
    let src = format!(
        "// @generated by `cargo xtask gen-proto`. Do not edit.\n\
         /// Decodes the proto3-JSON form of the message named by `type_url`\n\
         /// into protobuf bytes. `None` for unknown type URLs.\n\
         pub fn json_to_any_value(type_url: &str, value: serde_json::Value) -> Option<Result<Vec<u8>, String>> {{\n\
             Some(match type_url {{\n{decode}        _ => return None,\n    }})\n}}\n\n\
         /// Encodes protobuf bytes of the message named by `type_url` as\n\
         /// proto3 JSON. `None` for unknown type URLs.\n\
         pub fn any_value_to_json(type_url: &str, bytes: &[u8]) -> Option<Result<serde_json::Value, String>> {{\n\
             Some(match type_url {{\n{encode}        _ => return None,\n    }})\n}}\n"
    );
    std::fs::write(out.join("registry.rs"), src)?;
    Ok(())
}

fn rust_path(full_name: &str, package: &str) -> String {
    use heck::{ToSnakeCase as _, ToUpperCamelCase as _};
    let module: Vec<String> = package.split('.').map(|s| s.to_snake_case()).collect();
    let rest = &full_name[package.len() + 1..];
    let mut parts: Vec<&str> = rest.split('.').collect();
    let last = parts.pop().unwrap_or_default();
    let mut path = format!("crate::{}", module.join("::"));
    for p in parts {
        path.push_str("::");
        path.push_str(&p.to_snake_case());
    }
    format!("{path}::{}", last.to_upper_camel_case())
}

fn fn_name(full_name: &str) -> String {
    format!("signers_{}", full_name.replace('.', "_").to_lowercase())
}

/// Writes `signers.rs`: `cosmos.msg.v1.signer` extraction per message type.
fn write_signers(
    out: &Path,
    pool: &prost_reflect::DescriptorPool,
    packages: &std::collections::BTreeSet<String>,
) -> Result<()> {
    use heck::ToSnakeCase as _;
    use prost_reflect::{Kind, Value};
    use std::fmt::Write as _;
    let Some(ext) = pool.get_extension_by_name("cosmos.msg.v1.signer") else {
        anyhow::bail!("cosmos.msg.v1.signer extension not found");
    };
    let signer_fields = |m: &prost_reflect::MessageDescriptor| -> Vec<String> {
        let opts = m.options();
        match opts.get_extension(&ext).as_ref() {
            Value::List(l) => l
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect(),
            _ => Vec::new(),
        }
    };
    let mut fns = String::new();
    let mut arms = String::new();
    let mut todo: Vec<prost_reflect::MessageDescriptor> = pool
        .all_messages()
        .filter(|m| packages.contains(m.package_name()) && !signer_fields(m).is_empty())
        .collect();
    let mut done = std::collections::BTreeSet::new();
    let roots: Vec<String> = todo.iter().map(|m| m.full_name().to_owned()).collect();
    while let Some(m) = todo.pop() {
        if !done.insert(m.full_name().to_owned()) {
            continue;
        }
        let ty = rust_path(m.full_name(), m.package_name());
        let mut body = String::new();
        for fname in signer_fields(&m) {
            let Some(f) = m.get_field_by_name(&fname) else {
                anyhow::bail!("{}: signer field {fname} missing", m.full_name());
            };
            let ident = match fname.to_snake_case().as_str() {
                "type" => "r#type".to_owned(),
                other => other.to_owned(),
            };
            match (f.kind(), f.is_list()) {
                (Kind::String, false) => {
                    writeln!(body, "    out.push(Signer::Str(m.{ident}.clone()));")?
                }
                (Kind::String, true) => writeln!(
                    body,
                    "    for s in &m.{ident} {{ out.push(Signer::Str(s.clone())); }}"
                )?,
                (Kind::Bytes, false) => {
                    writeln!(body, "    out.push(Signer::Raw(m.{ident}.clone()));")?
                }
                (Kind::Message(inner), list) => {
                    let inner_fn = fn_name(inner.full_name());
                    if list {
                        writeln!(body, "    for x in &m.{ident} {{ {inner_fn}(x, out); }}")?;
                    } else {
                        writeln!(
                            body,
                            "    if let Some(x) = &m.{ident} {{ {inner_fn}(x, out); }}"
                        )?;
                    }
                    todo.push(inner);
                }
                (k, l) => anyhow::bail!(
                    "{}: unsupported signer field kind {k:?} list={l}",
                    m.full_name()
                ),
            }
        }
        writeln!(
            fns,
            "fn {}(m: &{ty}, out: &mut Vec<Signer>) {{\n{body}}}\n",
            fn_name(m.full_name())
        )?;
    }
    let mut sorted_roots = roots;
    sorted_roots.sort();
    for full in &sorted_roots {
        let Some(m) = pool.get_message_by_name(full) else {
            continue;
        };
        let ty = rust_path(m.full_name(), m.package_name());
        writeln!(
            arms,
            "        \"/{full}\" => {{ let m = match <{ty} as prost::Message>::decode(bytes) {{ Ok(m) => m, Err(e) => return Some(Err(e)) }}; let mut out = Vec::new(); {}(&m, &mut out); out }}",
            fn_name(full)
        )?;
    }
    let src = format!(
        "// @generated by `cargo xtask gen-proto`. Do not edit.\n\n{fns}\
         /// Signers of the message named by `type_url` (per its\n\
         /// `cosmos.msg.v1.signer` option). `None` for messages without one.\n\
         pub fn get_signers(type_url: &str, bytes: &[u8]) -> Option<Result<Vec<Signer>, prost::DecodeError>> {{\n\
             Some(Ok(match type_url {{\n{arms}        _ => return None,\n    }}))\n}}\n"
    );
    std::fs::write(out.join("signers.rs"), src)?;
    Ok(())
}

fn collect_protos(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            collect_protos(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "proto") {
            out.push(path);
        }
    }
    Ok(())
}

fn repo_root() -> Result<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    Ok(manifest
        .parent()
        .context("xtask has a parent dir")?
        .to_path_buf())
}
