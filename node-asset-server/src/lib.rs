//! One-way, versioned asset synchronization. Hashes describe logical content,
//! independent of redb page layout. The index uses prefix-compressed sorted keys.
use picasso::Picasso;
use serde::Deserialize;
use sha2::{Digest, Sha256};
#[cfg(not(target_os = "trueos"))]
pub mod server;

use std::{
    collections::BTreeSet,
    io::{self, Read, Write},
};

pub const PORT: u16 = 14002;
pub const HELLO: &[u8; 4] = b"VAS1";
pub const GET: &[u8; 4] = b"GET1";
pub const CATALOG: &str = "__veloren_asset_catalog/v1";
pub const MAX_FILES: usize = 16_385;
pub const MAX_INDEX: usize = 4 * 1024 * 1024;
pub const MAX_IMAGE: usize = 1024 * 1024 * 1024;
pub const MAX_BYTES: u64 = 520 * 1024 * 1024;
pub const MAX_ASSET: u64 = 128 * 1024 * 1024;
pub type Hash = [u8; 32];

pub fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}
fn storage(e: picasso::PicassoError) -> io::Error {
    io::Error::other(e.to_string())
}
pub fn hash(bytes: &[u8]) -> Hash {
    Sha256::digest(bytes).into()
}
pub fn lz4_encoder<W: Write>(output: W) -> lz4_flex::frame::FrameEncoder<W> {
    lz4_flex::frame::FrameEncoder::new(output)
}

/// Accept the existing raw image and its LZ4-packed deployment form.
pub fn decode_image(bytes: Vec<u8>) -> io::Result<Vec<u8>> {
    if bytes.starts_with(&[0x04, 0x22, 0x4d, 0x18]) {
        let mut image = Vec::new();
        lz4_flex::frame::FrameDecoder::new(bytes.as_slice())
            .take(MAX_IMAGE as u64 + 1)
            .read_to_end(&mut image)?;
        if image.len() > MAX_IMAGE {
            return Err(invalid("database exceeds limit"));
        }
        Ok(image)
    } else if bytes.len() > MAX_IMAGE {
        Err(invalid("database exceeds limit"))
    } else {
        Ok(bytes)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub key: String,
    pub len: u64,
    pub hash: Hash,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Index {
    pub root: Hash,
    pub files: Vec<File>,
}

impl Index {
    pub fn new(files: Vec<File>) -> io::Result<Self> {
        if files.len() > MAX_FILES {
            return Err(invalid("too many assets"));
        }
        let mut digest = Sha256::new();
        digest.update(HELLO);
        digest.update((files.len() as u32).to_le_bytes());
        let mut previous = "";
        let mut total = 0u64;
        for file in &files {
            if file.key.is_empty()
                || file.key.len() > 1024
                || file.key <= previous.to_owned()
                || file.key.contains('\0')
                || file.len > MAX_ASSET
            {
                return Err(invalid("invalid asset index entry"));
            }
            total = total
                .checked_add(file.len)
                .ok_or_else(|| invalid("asset size overflow"))?;
            if total > MAX_BYTES {
                return Err(invalid("asset database exceeds size limit"));
            }
            digest.update((file.key.len() as u32).to_le_bytes());
            digest.update(file.key.as_bytes());
            digest.update(file.len.to_le_bytes());
            digest.update(file.hash);
            previous = &file.key;
        }
        Ok(Self {
            root: digest.finalize().into(),
            files,
        })
    }

    pub fn encode(&self) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.root);
        out.extend_from_slice(&(self.files.len() as u32).to_le_bytes());
        let mut previous: &[u8] = &[];
        for file in &self.files {
            let key = file.key.as_bytes();
            let prefix = previous.iter().zip(key).take_while(|(a, b)| a == b).count();
            out.extend_from_slice(&(prefix as u16).to_le_bytes());
            out.extend_from_slice(&((key.len() - prefix) as u16).to_le_bytes());
            out.extend_from_slice(&key[prefix..]);
            out.extend_from_slice(&file.len.to_le_bytes());
            out.extend_from_slice(&file.hash);
            previous = key;
        }
        if out.len() > MAX_INDEX {
            return Err(invalid("index exceeds size limit"));
        }
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() > MAX_INDEX {
            return Err(invalid("index exceeds size limit"));
        }
        let mut input = bytes;
        let mut root = [0; 32];
        input.read_exact(&mut root)?;
        let count = u32_read(&mut input)? as usize;
        if count > MAX_FILES {
            return Err(invalid("too many assets"));
        }
        let mut files: Vec<File> = Vec::with_capacity(count);
        for _ in 0..count {
            let mut sizes = [0; 4];
            input.read_exact(&mut sizes)?;
            let prefix = u16::from_le_bytes(sizes[..2].try_into().unwrap()) as usize;
            let suffix = u16::from_le_bytes(sizes[2..].try_into().unwrap()) as usize;
            let previous = files.last().map_or(&b""[..], |f| f.key.as_bytes());
            if prefix > previous.len() || prefix + suffix > 1024 {
                return Err(invalid("invalid key prefix"));
            }
            let mut key = previous[..prefix].to_vec();
            key.resize(prefix + suffix, 0);
            input.read_exact(&mut key[prefix..])?;
            let len = u64_read(&mut input)?;
            let mut hash = [0; 32];
            input.read_exact(&mut hash)?;
            files.push(File {
                key: String::from_utf8(key).map_err(|_| invalid("invalid key UTF-8"))?,
                len,
                hash,
            });
        }
        let index = Self::new(files)?;
        if index.root != root || !input.is_empty() {
            return Err(invalid("invalid index hash or trailing data"));
        }
        Ok(index)
    }
}

pub struct AssetDb {
    pub store: Picasso,
    pub index: Index,
}
impl AssetDb {
    pub fn open(image: Vec<u8>) -> io::Result<Self> {
        if image.len() > MAX_IMAGE {
            return Err(invalid("database exceeds size limit"));
        }
        Self::from_store(
            Picasso::from_runtime_database_image(image).map_err(storage)?,
            &mut |_| Ok(()),
        )
    }
    pub fn from_store(
        store: Picasso,
        progress: &mut impl FnMut(&str) -> io::Result<()>,
    ) -> io::Result<Self> {
        let names = store.embedded_asset_names().map_err(storage)?;
        if names.len() > MAX_FILES {
            return Err(invalid("too many assets"));
        }
        let mut files = Vec::with_capacity(names.len());
        for key in names {
            progress(&key)?;
            let bytes = store
                .embedded_asset(&key)
                .map_err(storage)?
                .ok_or_else(|| invalid("missing asset"))?;
            files.push(File {
                key,
                len: bytes.len() as u64,
                hash: hash(&bytes),
            });
        }
        let db = Self {
            store,
            index: Index::new(files)?,
        };
        db.validate_catalog()?;
        Ok(db)
    }

    /// Delete obsolete entries before requesting any download. Deletion and
    /// replacement affect only the independent RAM work image.
    pub fn prune_and_diff(
        &mut self,
        target: &Index,
        progress: &mut impl FnMut(&str) -> io::Result<()>,
    ) -> io::Result<Vec<u32>> {
        let keep: BTreeSet<_> = target.files.iter().map(|f| f.key.as_str()).collect();
        for file in &self.index.files {
            if !keep.contains(file.key.as_str()) {
                progress(&file.key)?;
                self.store
                    .remove_embedded_asset(&file.key)
                    .map_err(storage)?;
            }
        }
        Ok(target
            .files
            .iter()
            .enumerate()
            .filter_map(|(i, file)| {
                let old = self
                    .index
                    .files
                    .binary_search_by(|f| f.key.cmp(&file.key))
                    .ok()
                    .map(|j| &self.index.files[j]);
                (old != Some(file)).then_some(i as u32)
            })
            .collect())
    }

    pub fn write_bundle(&self, indices: &[u32], output: impl Write) -> io::Result<()> {
        validate_request(&self.index, indices)?;
        let mut zip = lz4_flex::frame::FrameEncoder::new(output);
        zip.write_all(&(indices.len() as u32).to_le_bytes())?;
        for &i in indices {
            let file = &self.index.files[i as usize];
            let bytes = self
                .store
                .embedded_asset(&file.key)
                .map_err(storage)?
                .ok_or_else(|| invalid("missing asset"))?;
            zip.write_all(&i.to_le_bytes())?;
            zip.write_all(&file.len.to_le_bytes())?;
            zip.write_all(&bytes)?;
        }
        zip.finish().map_err(io::Error::other)?;
        Ok(())
    }

    pub fn apply_bundle(
        &mut self,
        target: &Index,
        indices: &[u32],
        compressed: &[u8],
        progress: &mut impl FnMut(&str) -> io::Result<()>,
    ) -> io::Result<()> {
        validate_request(target, indices)?;
        let mut zip = lz4_flex::frame::FrameDecoder::new(compressed);
        if u32_read(&mut zip)? as usize != indices.len() {
            return Err(invalid("bundle count mismatch"));
        }
        for &expected in indices {
            if u32_read(&mut zip)? != expected {
                return Err(invalid("bundle index mismatch"));
            }
            let file = &target.files[expected as usize];
            progress(&file.key)?;
            let len = u64_read(&mut zip)?;
            if len != file.len {
                return Err(invalid("bundle length mismatch"));
            }
            let mut bytes =
                vec![0; usize::try_from(len).map_err(|_| invalid("asset size overflow"))?];
            zip.read_exact(&mut bytes)?;
            if hash(&bytes) != file.hash {
                return Err(invalid("asset hash mismatch"));
            }
            self.store
                .put_embedded_asset(&file.key, &bytes)
                .map_err(storage)?;
        }
        let mut extra = [0];
        if zip.read(&mut extra)? != 0 {
            return Err(invalid("bundle has trailing data"));
        }
        self.index = target.clone();
        self.validate_catalog()
    }

    pub fn into_image(self) -> io::Result<Vec<u8>> {
        let image = self.store.into_runtime_database_image().map_err(storage)?;
        if image.len() > MAX_IMAGE {
            return Err(invalid("updated image exceeds size limit"));
        }
        Ok(image)
    }

    pub fn validate_catalog(&self) -> io::Result<()> {
        #[derive(Deserialize)]
        enum Entry {
            File(String, String),
            Directory(String),
        }
        #[derive(Deserialize)]
        struct Catalog {
            version: u32,
            directories: std::collections::BTreeMap<String, Vec<Entry>>,
            files: BTreeSet<String>,
            bytes: u64,
        }
        let raw = self
            .store
            .embedded_asset(CATALOG)
            .map_err(storage)?
            .ok_or_else(|| invalid("database has no catalog"))?;
        if raw.len() > 8 * 1024 * 1024 {
            return Err(invalid("catalog too large"));
        }
        let catalog: Catalog = ron::de::from_bytes(&raw).map_err(io::Error::other)?;
        let actual: BTreeSet<_> = self
            .index
            .files
            .iter()
            .filter(|f| f.key != CATALOG)
            .map(|f| f.key.clone())
            .collect();
        let total: u64 = self
            .index
            .files
            .iter()
            .filter(|f| f.key != CATALOG)
            .map(|f| f.len)
            .sum();
        if catalog.version != 1
            || catalog.files != actual
            || catalog.bytes != total
            || !catalog.directories.contains_key("")
        {
            return Err(invalid("catalog does not match assets"));
        }
        let mut listed = BTreeSet::new();
        for (dir, entries) in &catalog.directories {
            for entry in entries {
                match entry {
                    Entry::File(id, ext) => {
                        if id.rsplit_once('.').map_or("", |(parent, _)| parent) != dir
                            || id.contains('/')
                            || ext.contains('/')
                            || !listed.insert(format!("{id}/{ext}"))
                        {
                            return Err(invalid("invalid catalog file"));
                        }
                    }
                    Entry::Directory(id) => {
                        if !catalog.directories.contains_key(id)
                            || id.rsplit_once('.').map_or("", |(p, _)| p) != dir
                            || id == dir
                        {
                            return Err(invalid("invalid catalog directory"));
                        }
                    }
                }
            }
        }
        if listed != actual {
            return Err(invalid("incomplete catalog"));
        }
        let canary = self
            .store
            .embedded_asset("common.canary/canary")
            .map_err(storage)?
            .ok_or_else(|| invalid("missing canary"))?;
        if !canary.starts_with(b"VELOREN_CANARY_MAGIC") {
            return Err(invalid("invalid canary"));
        }
        Ok(())
    }
}

pub fn validate_request(index: &Index, indices: &[u32]) -> io::Result<()> {
    if indices.len() > MAX_FILES
        || indices.iter().any(|i| *i as usize >= index.files.len())
        || indices.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(invalid("invalid bundle indices"));
    }
    Ok(())
}
pub fn u32_read(input: &mut impl Read) -> io::Result<u32> {
    let mut b = [0; 4];
    input.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}
pub fn u64_read(input: &mut impl Read) -> io::Result<u64> {
    let mut b = [0; 8];
    input.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database(entries: &[(&str, &[u8])]) -> AssetDb {
        let store = Picasso::new().unwrap();
        store
            .put_embedded_asset("common.canary/canary", b"VELOREN_CANARY_MAGIC")
            .unwrap();
        let mut files = vec!["common.canary/canary".to_owned()];
        let mut bytes = b"VELOREN_CANARY_MAGIC".len();
        let mut root = Vec::new();
        for (key, value) in entries {
            store.put_embedded_asset(key, value).unwrap();
            files.push((*key).to_owned());
            bytes += value.len();
            let (id, ext) = key.split_once('/').unwrap();
            root.push(format!("File({id:?},{ext:?})"));
        }
        files.sort();
        let catalog = format!(
            "(version:1,directories:{{\"\":[Directory(\"common\"),{}],\"common\":[File(\"common.canary\",\"canary\")]}},files:{files:?},bytes:{bytes})",
            root.join(",")
        );
        store
            .put_embedded_asset(CATALOG, catalog.as_bytes())
            .unwrap();
        AssetDb::from_store(store, &mut |_| Ok(())).unwrap()
    }

    #[test]
    fn delta_removes_replaces_adds_and_preserves_unchanged_bytes() {
        let old = database(&[
            ("gone/bin", &[7; 180_000]),
            ("change/bin", b"old"),
            ("keep/bin", b"same"),
        ]);
        let serving = AssetDb::open(old.into_image().unwrap()).unwrap();
        let target = database(&[
            ("change/bin", b"new"),
            ("empty/bin", b""),
            ("keep/bin", b"same"),
            ("large/bin", &[8; 180_000]),
        ]);
        let mut local =
            AssetDb::from_store(serving.store.fork_runtime_database().unwrap(), &mut |_| {
                Ok(())
            })
            .unwrap();
        let index = Index::decode(&target.index.encode().unwrap()).unwrap();
        let diff = local.prune_and_diff(&index, &mut |_| Ok(())).unwrap();
        assert!(local.store.embedded_asset("gone/bin").unwrap().is_none());
        assert!(serving.store.embedded_asset("gone/bin").unwrap().is_some());
        assert!(
            !diff
                .iter()
                .any(|i| index.files[*i as usize].key == "keep/bin")
        );
        let mut bundle = Vec::new();
        target.write_bundle(&diff, &mut bundle).unwrap();
        local
            .apply_bundle(&index, &diff, &bundle, &mut |_| Ok(()))
            .unwrap();
        let reopened = AssetDb::open(local.into_image().unwrap()).unwrap();
        assert_eq!(reopened.index, target.index);
        assert_eq!(
            reopened
                .store
                .embedded_asset("change/bin")
                .unwrap()
                .unwrap(),
            b"new"
        );
        assert_eq!(
            reopened.store.embedded_asset("empty/bin").unwrap().unwrap(),
            b""
        );
        assert_eq!(
            serving.store.embedded_asset("change/bin").unwrap().unwrap(),
            b"old"
        );
    }

    #[test]
    fn root_ignores_redb_layout_and_insertion_order() {
        let a = database(&[("a/bin", b"a"), ("b/bin", b"b")]);
        let b = database(&[("b/bin", b"b"), ("a/bin", b"a")]);
        // Catalog byte ordering is content too; compare noncatalog files here.
        let a = Index::new(
            a.index
                .files
                .into_iter()
                .filter(|f| f.key != CATALOG)
                .collect(),
        )
        .unwrap();
        let b = Index::new(
            b.index
                .files
                .into_iter()
                .filter(|f| f.key != CATALOG)
                .collect(),
        )
        .unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn rejects_corrupt_indexes_duplicate_requests_and_truncated_streams() {
        let db = database(&[("a/bin", b"a")]);
        let mut index = db.index.encode().unwrap();
        index[0] ^= 1;
        assert!(Index::decode(&index).is_err());
        assert!(db.write_bundle(&[0, 0], Vec::new()).is_err());
        assert!(db.write_bundle(&[u32::MAX], Vec::new()).is_err());
        let mut working =
            AssetDb::from_store(db.store.fork_runtime_database().unwrap(), &mut |_| Ok(()))
                .unwrap();
        let diff = vec![0];
        let mut zip = Vec::new();
        db.write_bundle(&diff, &mut zip).unwrap();
        zip.truncate(zip.len() / 2);
        assert!(
            working
                .apply_bundle(&db.index, &diff, &zip, &mut |_| Ok(()))
                .is_err()
        );
        let mut empty = Vec::new();
        db.write_bundle(&[], &mut empty).unwrap();
        working
            .apply_bundle(&db.index, &[], &empty, &mut |_| Ok(()))
            .unwrap();
    }

    #[test]
    fn cancellation_while_pruning_leaves_the_original_intact() {
        let old = database(&[("gone/bin", b"gone")]);
        let target = database(&[]);
        let mut fork =
            AssetDb::from_store(old.store.fork_runtime_database().unwrap(), &mut |_| Ok(()))
                .unwrap();
        assert!(
            fork.prune_and_diff(&target.index, &mut |_| Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "canceled"
            )))
            .is_err()
        );
        assert_eq!(
            old.store.embedded_asset("gone/bin").unwrap().unwrap(),
            b"gone"
        );
    }
}
