# Asset image builder

From the asset repository, run:

```sh
./build-assets.sh
```

This imports the repository's raw asset tree into `voxygen-assets.redb`, then
packs `voxygen-assets.redb.lz4` using `lz4 -B4`. Both output files are replaced.
The node server and builder tooling, launcher, and generated images are excluded
from ingestion. The source directory is explicit: no `assets` symlink is needed.

An optional output path is relative to the asset repository:

```sh
./build-assets.sh /tmp/voxygen-assets.redb
```

Set `VELOREN_ASSETS` to import a different source directory. Its path is used
directly. `VELOREN_ASSETS_OVERRIDE` retains the game's optional overlay behavior.
`VOXY_ASSET_TOOLCHAIN` can select another Rust toolchain; the default is
`nightly-2026-07-10`. Requirements are Cargo/Rust, the `lz4` executable, and sibling
`voxy` and `TRUEOS-Picasso` checkouts. The builder shares the game's importer and
catalog format rather than maintaining a separate image format.

After rebuilding, restart `node-asset-server` so it loads the new image into RAM.
Clients can then request the changes using **Asset Sync**. A rebuild does not
update an already running node or publish the image automatically.
