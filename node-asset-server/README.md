# Voxy node asset server

The asset node listens on **TCP 14002**. It reads `../voxygen-assets.redb` once
on startup, opens the image as a read-only RAM database, and builds its index.
It never watches or rereads that image. Restart the process after replacing it.
The executable is standalone; its build shares Picasso and protocol code with
the Voxy client. Raw `.redb` and LZ4-packed images are both accepted.

```sh
cd /home/t4ce/Repos/voxy-assets/node-asset-server
cargo +nightly-2026-07-10 build --release
./target/release/node-asset-server
```

Optional arguments:

```sh
./target/release/node-asset-server --db ../voxygen-assets.redb --bind 0.0.0.0:14002
```

Voxy's **Asset Sync** button is above Quit. It reuses the selected game server's
host/address resolution and overrides only the port to 14002. Sync is manual;
there is no background polling. The game server and asset node use separate
TCP ports. No authentication or client uploads are involved.

The client keeps its serving RAM database intact while syncing an independent
writable RAM copy. It removes obsolete keys before sending the bundle request;
redb can reuse those pages for incoming assets. Only requested new/changed keys
are transmitted, including the directory catalog when needed. Hash validation
and catalog/canary checks happen before persistence. Cancel discards preparation
and leaves the installed disk image intact. Once the final disk write starts,
Cancel is hidden and window-close/Quit cannot interrupt the commit. TRUEOS uses
its native async typed atomic file replacement (`LZ4` identity); the host proof
uses a temporary file, fsync and rename. The disk target is
`VOXYGEN_ASSET_DATABASE`, default `/apps/voxy/voxygen-assets.redb.lz4`.

After commit, Voxy publishes the new raw RAM source. Already decoded objects and
renderer texture handles retain their current values; restart Voxy to refresh
those existing handles. Newly loaded raw assets use the updated source.

The node tooling directory is excluded from future asset-image ingestion.

## Wire protocol v1

All integers are little endian. Hashes are SHA-256 over canonical logical
content, not redb's physical page layout. The directory catalog is content too.

1. Client: `VAS1` + 32-byte local root hash.
2. Server: byte `0` for a match, then closes; or byte `1`, u32 index byte length,
   and the compact index.
3. Index: 32-byte root, u32 entry count; each sorted entry has u16 shared-prefix
   length, u16 suffix length, suffix UTF-8 bytes, u64 content length and SHA-256.
   Keys are Picasso asset IDs (`voxygen.voxel.object.portal/vox`).
4. After pruning its working copy, client: `GET1` + target root + u32 requested
   entry count + ascending u32 index IDs. No asset bytes are uploaded.
5. Server: byte `2`, then one LZ4 frame, streaming directly from its RAM database
   until connection close. Decompressed body: u32 count, followed by each u32
   index ID, u64 length and asset bytes. Every length and per-file hash must
   match the previously validated index. Stale roots, duplicate IDs, oversized
   indexes/bundles and corrupt/incomplete updates are rejected.

Limits: 16,385 records including catalog, 4 MiB index, 520 MiB logical content,
128 MiB per asset, 1 GiB database image, eight concurrent connections. Idle reads
time out after five minutes (allowing client pruning); blocked writes after
30 seconds. A dropped connection never changes the node snapshot.

## Verification

```sh
cargo +nightly-2026-07-10 test --offline
cd /home/t4ce/Repos/voxy
cargo +nightly-2026-07-10 test --manifest-path tests/native-asset-sync/Cargo.toml --offline -- --test-threads=1
```

The host harness runs the production Voxy worker with actual TCP and redb,
checks durable replacement, removal/addition, the matching-root fast path,
fresh raw-source reads, and races cancellation against the write boundary.
