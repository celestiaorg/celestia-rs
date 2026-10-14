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
        let fds =
            protox::compile(&files, &includes).map_err(|e| anyhow::anyhow!("protox: {e:?}"))?;
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
