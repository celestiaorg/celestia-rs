//! Debug-print selected recorded exchanges: `peek <scenario> <index>...`.
#![allow(clippy::print_stdout)]
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let ex = devnet_conformance::load_exchanges(&args[1])?;
    for i in &args[2..] {
        let i: usize = i.parse()?;
        println!("#{i} {:#?}", ex[i]);
    }
    Ok(())
}
