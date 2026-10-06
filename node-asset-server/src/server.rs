use crate::{AssetDb, GET, HELLO, MAX_FILES, invalid, u32_read, validate_request};
use std::{
    io::{self, Read, Write},
    net::TcpStream,
    time::Duration,
};

pub fn serve(mut socket: TcpStream, db: &AssetDb, index: &[u8]) -> io::Result<()> {
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
