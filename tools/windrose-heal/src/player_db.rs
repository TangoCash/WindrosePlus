use anyhow::{anyhow, Context, Result};
use bson::{Bson, Document};
use rocksdb::{ColumnFamilyDescriptor, Options, DB};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Expected column families in the Players/<PlayerId> RocksDB.
pub const CFS: &[&str] = &[
    "default",
    "R5LargeObjects",
    "R5BLPlayer",
    "R5BLShip",
    "R5BLBuilding",
    "R5BLActor_BuildingBlock",
];

/// Resolve `input` to the actual Players/<PlayerId> RocksDB dir.
/// Accepts either that dir directly, or a SaveProfiles/<steamid> tree which we'll walk.
pub fn resolve_player_db(input: &Path) -> Result<PathBuf> {
    if !input.exists() {
        anyhow::bail!("path does not exist: {:?}", input);
    }

    // If this is already a Players/<PlayerId> dir (has a CURRENT file and sibling named with hex id)
    if input.join("CURRENT").exists() {
        return Ok(input.to_path_buf());
    }

    // Walk common subpaths
    let candidates = ["RocksDB/0.10.0/Players", "RocksDB\\0.10.0\\Players"];
    for c in candidates.iter() {
        let p = input.join(c);
        if p.is_dir() {
            // Find the single PlayerId subdir
            let entries: Vec<_> = std::fs::read_dir(&p)?
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .collect();
            match entries.len() {
                0 => anyhow::bail!("no PlayerId subdirs in {:?}", p),
                1 => return Ok(entries[0].path()),
                n => anyhow::bail!("expected 1 PlayerId subdir in {:?}, found {}", p, n),
            }
        }
    }

    anyhow::bail!(
        "could not resolve Players RocksDB from {:?}; pass the Players/<PlayerId> dir directly",
        input
    );
}

fn open_db(path: &Path, read_only: bool) -> Result<DB> {
    let mut opts = Options::default();
    opts.create_if_missing(false);
    opts.create_missing_column_families(false);

    let cfs: Vec<ColumnFamilyDescriptor> = CFS
        .iter()
        .map(|name| ColumnFamilyDescriptor::new(*name, Options::default()))
        .collect();

    if read_only {
        DB::open_cf_descriptors_read_only(&opts, path, cfs, false)
            .with_context(|| format!("open read-only {:?}", path))
    } else {
        DB::open_cf_descriptors(&opts, path, cfs)
            .with_context(|| format!("open read-write {:?}", path))
    }
}

/// Read the single expected row from column family R5BLPlayer. Returns (key_bytes, value_bytes).
pub fn read_single_r5bl_player_row(db_path: &Path) -> Result<(Vec<u8>, Vec<u8>)> {
    let db = open_db(db_path, true)?;
    let cf = db
        .cf_handle("R5BLPlayer")
        .ok_or_else(|| anyhow!("column family R5BLPlayer missing"))?;

    let iter = db.iterator_cf(&cf, rocksdb::IteratorMode::Start);
    let mut rows: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    for entry in iter {
        let (k, v) = entry.context("iterating R5BLPlayer")?;
        rows.push((k.to_vec(), v.to_vec()));
    }

    match rows.len() {
        0 => Err(anyhow!("R5BLPlayer is empty — nothing to heal")),
        1 => Ok(rows.into_iter().next().unwrap()),
        n => Err(anyhow!(
            "expected exactly 1 row in R5BLPlayer, found {}. Refusing to repair.",
            n
        )),
    }
}

/// Write the repaired value back to R5BLPlayer at the given key.
pub fn write_r5bl_player_row(db_path: &Path, key: &[u8], value: &[u8]) -> Result<()> {
    let db = open_db(db_path, false)?;
    let cf = db
        .cf_handle("R5BLPlayer")
        .ok_or_else(|| anyhow!("column family R5BLPlayer missing"))?;
    db.put_cf(&cf, key, value).context("put_cf R5BLPlayer")?;
    db.flush().context("flush after write")?;
    Ok(())
}

/// Parse BSON bytes into a Document, preserving field order.
pub fn parse_bson(bytes: &[u8]) -> Result<Document> {
    let mut cursor = std::io::Cursor::new(bytes);
    let doc = Document::from_reader(&mut cursor).context("decode BSON")?;
    Ok(doc)
}

/// Emit Document back to BSON bytes.
pub fn emit_bson(doc: &Document) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    doc.to_writer(&mut out).context("encode BSON")?;
    Ok(out)
}

/// Assert the Document round-trips byte-identically through the parser.
/// If this fails, we can't safely edit — our parser is lossy for this shape.
pub fn assert_roundtrip(doc: &Document, original: &[u8]) -> Result<()> {
    let reemitted = emit_bson(doc)?;
    if reemitted != original {
        let orig_sha = sha_hex(original);
        let reem_sha = sha_hex(&reemitted);
        anyhow::bail!(
            "round-trip mismatch: original len={} sha256={} vs re-emitted len={} sha256={}",
            original.len(),
            orig_sha,
            reemitted.len(),
            reem_sha
        );
    }
    Ok(())
}

pub fn sha_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

fn walk_ref<'a>(doc: &'a Document, path: &str) -> Option<&'a Bson> {
    let mut parts = path.split('.');
    let first = parts.next()?;
    let mut cur = doc.get(first)?;
    for p in parts {
        match cur {
            Bson::Document(d) => {
                cur = d.get(p)?;
            }
            _ => return None,
        }
    }
    Some(cur)
}

/// Set a value at a nested path. Creates nothing — all intermediate docs must exist.
pub fn set_path_i32(doc: &mut Document, path: &str, value: i32) -> Result<()> {
    let parts: Vec<&str> = path.split('.').collect();
    let (leaf, parents) = parts.split_last().ok_or_else(|| anyhow!("empty path"))?;

    let mut cur: &mut Document = doc;
    for p in parents {
        let next = cur
            .get_mut(*p)
            .ok_or_else(|| anyhow!("missing intermediate: {}", p))?;
        cur = match next {
            Bson::Document(d) => d,
            _ => anyhow::bail!("path segment {} is not a document", p),
        };
    }

    let existing = cur
        .get(*leaf)
        .ok_or_else(|| anyhow!("leaf field missing: {}", leaf))?;
    // Preserve the original BSON int type width (i32 vs i64)
    let new_bson = match existing {
        Bson::Int32(_) => Bson::Int32(value),
        Bson::Int64(_) => Bson::Int64(value as i64),
        other => anyhow::bail!(
            "leaf {} is not an integer type: {:?}",
            leaf,
            other.element_type()
        ),
    };
    cur.insert((*leaf).to_string(), new_bson);
    Ok(())
}

pub fn get_i64(doc: &Document, path: &str) -> Result<i64> {
    let v = walk_ref(doc, path).ok_or_else(|| anyhow!("missing path: {}", path))?;
    match v {
        Bson::Int32(i) => Ok(*i as i64),
        Bson::Int64(i) => Ok(*i),
        other => anyhow::bail!(
            "path {} is not integer, got {:?}",
            path,
            other.element_type()
        ),
    }
}

pub fn get_string(doc: &Document, path: &str) -> Result<String> {
    let v = walk_ref(doc, path).ok_or_else(|| anyhow!("missing path: {}", path))?;
    match v {
        Bson::String(s) => Ok(s.clone()),
        other => anyhow::bail!(
            "path {} is not string, got {:?}",
            path,
            other.element_type()
        ),
    }
}
