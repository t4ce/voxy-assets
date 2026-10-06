use std::{
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use voxy_asset_sync::{AssetDb, GET, HELLO, MAX_FILES, invalid, u32_read, validate_request};

fn serve(mut socket: TcpStream, db: &AssetDb, index: &[u8]) -> io::Result<()> {
    socket.set_read_timeout(Some(Duration::from_secs(300)))?;
    socket.set_write_timeout(Some(Duration::from_secs(30)))?;
    let mut magic = [0; 4];
    socket.read_exact(&mut magic)?;
    if &magic != HELLO {
        return Err(invalid("unsupported asset-sync protocol"));
    }
    let mut root = [0; 32];
    socket.read_exact(&mut root)?;
    if root == db.index.root {
        socket.write_all(&[0])?;
        return Ok(());
    }
    socket.write_all(&[1])?;
    socket.write_all(&(index.len() as u32).to_le_bytes())?;
    socket.write_all(index)?;
    socket.read_exact(&mut magic)?;
    socket.read_exact(&mut root)?;
    if &magic != GET || root != db.index.root {
        return Err(invalid("stale or invalid bundle request"));
    }
    let count = u32_read(&mut socket)? as usize;
    if count > MAX_FILES {
        return Err(invalid("bundle request too large"));
    }
    let mut indices = Vec::with_capacity(count);
    for _ in 0..count {
        indices.push(u32_read(&mut socket)?);
    }
    validate_request(&db.index, &indices)?;
    socket.write_all(&[2])?;
    let mut output = io::BufWriter::with_capacity(64 * 1024, &mut socket);
    db.write_bundle(&indices, &mut output)?;
    output.flush()
}

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
    let db = Arc::new(AssetDb::open(voxy_asset_sync::decode_image(std::fs::read(path)?)?)?);
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
