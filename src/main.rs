use anyhow::Result;

fn main() -> Result<()> {
    println!("strategos v{}", env!("CARGO_PKG_VERSION"));
    Ok(())
}
