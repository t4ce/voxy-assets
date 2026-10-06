use std::{io, path::PathBuf, process::Command};

fn main() -> io::Result<()> {
    let assets = std::env::var_os("VELOREN_ASSETS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."));
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    if first
        .as_deref()
        .is_some_and(|arg| arg == "--help" || arg == "-h")
    {
        println!(
            "usage: voxy-assets [output.redb]\nSource: VELOREN_ASSETS or the voxy-assets repository root."
        );
        return Ok(());
    }
    let output = first
        .map(PathBuf::from)
        .unwrap_or_else(|| assets.join("voxygen-assets.redb"));
    if args.next().is_some() || output.extension().is_none_or(|ext| ext != "redb") {
        return Err(io::Error::other("usage: voxy-assets [output.redb]"));
    }
    veloren_common_assets::prepare_picasso_asset_database_from(&assets, &output)?;
    let compressed = output.with_extension("redb.lz4");
    let status = Command::new("lz4")
        .args(["-q", "-f", "-B4"])
        .arg(&output)
        .arg(&compressed)
        .status()?;
    if !status.success() {
        return Err(io::Error::other("LZ4 compression failed"));
    }
    eprintln!(
        "Voxygen assets: phase=db-packed path={} bytes={}",
        compressed.display(),
        std::fs::metadata(&compressed)?.len()
    );
    Ok(())
}
