#![allow(clippy::panic)]
//! Compiles the vendored `.proto` sources into a `FileDescriptorSet` that the
//! crate embeds. The legacy amino JSON sign mode (x/tx `aminojson`) and
//! signer extraction are driven by proto options (`amino.*`,
//! `cosmos_proto.scalar`), which only a descriptor pool carries.

use std::path::{Path, PathBuf};

const INCLUDES: &[&str] = &[
    "proto/cosmos-sdk",
    "proto/ibc-go",
    "proto/celestia-app",
    "proto/hyperlane-cosmos",
    "proto/packet-forward-middleware",
    "proto/celestia-core",
    "proto/third_party",
];

const SOURCES: &[&str] = &[
    "proto/cosmos-sdk",
    "proto/ibc-go",
    "proto/celestia-app",
    "proto/hyperlane-cosmos",
    "proto/packet-forward-middleware",
];

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "proto") {
            out.push(p);
        }
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let root = manifest
        .ancestors()
        .nth(4)
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let mut files = Vec::new();
    for s in SOURCES {
        collect(&root.join(s), &mut files);
    }
    files.sort();
    println!("cargo:rerun-if-changed=build.rs");
    for s in SOURCES {
        println!("cargo:rerun-if-changed={}", root.join(s).display());
    }
    let includes: Vec<PathBuf> = INCLUDES.iter().map(|i| root.join(i)).collect();
    let mut compiler = match protox::Compiler::new(&includes) {
        Ok(c) => c,
        Err(e) => panic!("protox: {e:?}"),
    };
    compiler.include_imports(true);
    compiler.include_source_info(false);
    if let Err(e) = compiler.open_files(&files) {
        panic!("protox: {e:?}");
    }
    let fds = compiler.file_descriptor_set();
    let bytes = prost::Message::encode_to_vec(&fds);
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default()).join("descriptors.bin");
    if let Err(e) = std::fs::write(&out, bytes) {
        panic!("write {}: {e}", out.display());
    }
}
