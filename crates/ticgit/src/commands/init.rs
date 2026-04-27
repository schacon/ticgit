use anyhow::Result;

use crate::commands::open_store;

pub fn run() -> Result<()> {
    let store = open_store()?;
    println!(
        "ticgit initialised on this repository (schema {}).",
        store.schema_version()?.unwrap_or_else(|| "?".into())
    );
    println!("Identity for new metadata: {}", store.email());
    Ok(())
}
