use std::{
    io,
    net::TcpListener,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use voxy_asset_sync::{AssetDb, invalid, server::serve};

fn main() -> io::Result<()> {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../voxygen-assets.redb");
    let mut bind = format!("0.0.0.0:{}", voxy_asset_sync::PORT);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--db" => {
                path = args
                    .next()
                    .ok_or_else(|| invalid("--db requires a path"))?
                    .into()
            }
            "--bind" => {
                bind = args
                    .next()
                    .ok_or_else(|| invalid("--bind requires an address"))?
            }
            _ => {
                return Err(invalid(
                    "usage: node-asset-server [--db image.redb] [--bind IP:PORT]",
                ));
            }
        }
    }
    eprintln!("Loading {} once into RAM...", path.display());
    let db = Arc::new(AssetDb::open(voxy_asset_sync::decode_image(
        std::fs::read(path)?,
    )?)?);
    let index = Arc::new(db.index.encode()?);
    let listener = TcpListener::bind(&bind)?;
    eprintln!(
        "Asset server {}: {} entries, index {} bytes, content hash {:02x?}",
        listener.local_addr()?,
        db.index.files.len(),
        index.len(),
        db.index.root
    );
    let active = Arc::new(AtomicUsize::new(0));
    for socket in listener.incoming() {
        let socket = socket?;
        if active.fetch_add(1, Ordering::AcqRel) >= 8 {
            active.fetch_sub(1, Ordering::AcqRel);
            continue;
        }
        let (db, index, active) = (Arc::clone(&db), Arc::clone(&index), Arc::clone(&active));
        std::thread::spawn(move || {
            if let Err(error) = serve(socket, &db, &index) {
                eprintln!("Asset request: {error}");
            }
            active.fetch_sub(1, Ordering::AcqRel);
        });
    }
    Ok(())
}
