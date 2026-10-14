//! Prints a replay report: `replay <scenario>`.
#![allow(clippy::print_stdout)]

fn printable(b: &[u8]) -> String {
    b.iter()
        .map(|c| {
            if c.is_ascii_graphic() {
                char::from(*c)
            } else {
                '.'
            }
        })
        .collect()
}

fn main() -> anyhow::Result<()> {
    let scenario = std::env::args().nth(1).unwrap_or_else(|| "empty".into());
    let r = devnet_conformance::replay::replay(&scenario)?;
    println!("matched heights: {:?}", r.matched_heights);
    println!("first mismatch: {:?}", r.first_mismatch);
    println!("halted at: {:?}", r.halted_at);
    println!("errors: {:?}", r.errors);
    println!("init_chain diffs: {:?}", r.init_chain_diffs);
    println!(
        "matching stores ({}): {:?}",
        r.matching_stores.len(),
        r.matching_stores
    );
    for d in &r.store_diffs {
        println!(
            "== {} missing={} extra={} different={} hash_only={}",
            d.name,
            d.missing.len(),
            d.extra.len(),
            d.different.len(),
            d.hash_only
        );
        for k in d.missing.iter().take(5) {
            println!("   - missing {} {}", hex::encode(k), printable(k));
        }
        for k in d.extra.iter().take(5) {
            println!("   + extra   {} {}", hex::encode(k), printable(k));
        }
        for (k, g, o) in d.different.iter().take(5) {
            println!(
                "   ~ {} go={} rust={}",
                hex::encode(k),
                hex::encode(g),
                hex::encode(o)
            );
        }
    }
    Ok(())
}
