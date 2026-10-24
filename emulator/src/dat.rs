//! Read-only AC2 BT DAT lookup. Mirrors tools/ac2_dat.py and recovered
//! BTNode/BTEntry / BTree::LoadTree (0064cbb0). No overlay or payload interpretation.
use std::{
    collections::HashSet,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};

const HEADER_OFFSET: u64 = 0x12c;
const HEADER_SIZE: usize = 0x50;
const NODE_SIZE: usize = 0x4cc;
const MAX_ENTRIES: usize = 61;
pub const MAX_RECORD_BYTES: usize = 64 * 1024 * 1024;
const MAX_TREE_DEPTH: usize = 64;
const MAX_VISITED_NODES: usize = 100_000;

/// Outdoor cells are synthesized from their block terrain, not separate DAT
/// records. The supported profile has 8x8 outdoor cells and envcells >= 0x100.
/// This identifies backing data; it does not establish traversability.
pub fn world_cell_record(cell: u32) -> io::Result<u32> {
    match cell & 0xffff {
        1..=64 => Ok(cell | 0xffff),
        0x100..=0xfffb => Ok(cell),
        _ => Err(invalid("invalid world cell or reserved record identifier")),
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordInfo {
    pub id: u32,
    pub offset: u32,
    pub length: u32,
    pub stamp: u32,
}

pub struct Dat {
    file: File,
    size: u64,
    block_size: u32,
    root: u32,
}
impl Dat {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let mut file = File::open(path)?;
        let size = file.metadata()?.len();
        file.seek(SeekFrom::Start(HEADER_OFFSET))?;
        let mut header = [0; HEADER_SIZE];
        file.read_exact(&mut header)
            .map_err(|e| io::Error::new(e.kind(), format!("cannot read AC2 DAT header: {e}")))?;
        if word(&header, 0) != 0x5442 {
            return Err(invalid("unsupported DAT: expected BT magic"));
        }
        let block_size = word(&header, 4);
        if !(256..=65536).contains(&block_size) {
            return Err(invalid("invalid DAT block size (expected 256..65536)"));
        }
        let dat = Self {
            file,
            size,
            block_size,
            root: word(&header, 0x20),
        };
        dat.validate_offset(dat.root)?;
        Ok(dat)
    }
    pub fn block_size(&self) -> u32 {
        self.block_size
    }
    fn validate_offset(&self, offset: u32) -> io::Result<()> {
        if offset < self.block_size || offset % self.block_size != 0 {
            return Err(invalid(format!("invalid DAT block offset 0x{offset:08x}")));
        }
        if u64::from(offset) + u64::from(self.block_size) > self.size {
            return Err(invalid(format!(
                "DAT block outside archive: 0x{offset:08x}"
            )));
        }
        Ok(())
    }
    fn chain(&mut self, mut offset: u32, length: usize) -> io::Result<Vec<u8>> {
        if length > MAX_RECORD_BYTES || length as u64 > self.size {
            return Err(invalid("DAT record exceeds archive size or 64 MiB limit"));
        }
        let mut result = Vec::new();
        result
            .try_reserve_exact(length)
            .map_err(|_| invalid("cannot allocate bounded DAT record"))?;
        let mut seen = HashSet::new();
        let mut block = vec![0; self.block_size as usize];
        while result.len() < length {
            self.validate_offset(offset)?;
            if !seen.insert(offset) {
                return Err(invalid(format!("cyclic DAT block chain at 0x{offset:08x}")));
            }
            self.file.seek(SeekFrom::Start(u64::from(offset)))?;
            self.file.read_exact(&mut block).map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!("cannot read DAT block 0x{offset:08x}: {e}"),
                )
            })?;
            offset = word(&block, 0);
            let take = (length - result.len()).min(block.len() - 4);
            result.extend_from_slice(&block[4..4 + take]);
        }
        Ok(result)
    }
    /// Visit only the sorted search path; corruption elsewhere is not a whole-archive audit.
    pub fn find(&mut self, id: u32) -> io::Result<Option<RecordInfo>> {
        let mut offset = self.root;
        let mut seen = HashSet::new();
        let (mut lower, mut upper) = (None, None);
        for _ in 0..MAX_TREE_DEPTH {
            if seen.len() >= MAX_VISITED_NODES {
                return Err(invalid("DAT B-tree node limit exceeded"));
            }
            if !seen.insert(offset) {
                return Err(invalid("cyclic DAT B-tree"));
            }
            let node = self.chain(offset, NODE_SIZE)?;
            let count = word(&node, 248) as usize;
            if count > MAX_ENTRIES {
                return Err(invalid("invalid DAT B-tree entry count"));
            }
            let entries: Vec<RecordInfo> = (0..count)
                .map(|i| {
                    let p = 252 + i * 16;
                    RecordInfo {
                        id: word(&node, p),
                        offset: word(&node, p + 4),
                        length: word(&node, p + 8),
                        stamp: word(&node, p + 12),
                    }
                })
                .collect();
            let mut previous = lower;
            for e in &entries {
                if previous.is_some_and(|n| e.id <= n) || upper.is_some_and(|n| e.id >= n) {
                    return Err(invalid("unsorted, duplicate or out-of-range DAT B-tree ID"));
                }
                previous = Some(e.id);
            }
            let leaf = word(&node, 0) == 0;
            // Native LoadTree identifies leaves solely by child[0]. Remaining
            // leaf child slots are unused and can retain stale archive bytes.
            if !leaf {
                for i in 0..=count {
                    self.validate_offset(word(&node, i * 4))?;
                }
            }
            match entries.binary_search_by_key(&id, |e| e.id) {
                Ok(i) => return Ok(Some(entries[i])),
                Err(i) => {
                    if leaf {
                        return Ok(None);
                    }
                    if i > 0 {
                        lower = Some(entries[i - 1].id);
                    }
                    if i < count {
                        upper = Some(entries[i].id);
                    }
                    offset = word(&node, i * 4);
                }
            }
        }
        Err(invalid("DAT B-tree depth exceeds 64"))
    }
    pub fn read(&mut self, id: u32) -> io::Result<Vec<u8>> {
        let entry = self.find(id)?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("DAT record 0x{id:08x} not found"),
            )
        })?;
        self.chain(entry.offset, entry.length as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    const BLOCK: usize = 512;
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn write(bytes: &[u8]) -> Self {
            let p = std::env::temp_dir().join(format!(
                "ac2-dat-test-{}-{}.dat",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::write(&p, bytes).unwrap();
            Self(p)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    fn put(bytes: &mut [u8], at: usize, n: u32) {
        bytes[at..at + 4].copy_from_slice(&n.to_le_bytes());
    }
    fn chain(bytes: &mut [u8], start: usize, payload: &[u8]) {
        for (i, chunk) in payload.chunks(BLOCK - 4).enumerate() {
            let at = start + i * BLOCK;
            let next = if (i + 1) * (BLOCK - 4) < payload.len() {
                (at + BLOCK) as u32
            } else {
                0
            };
            put(bytes, at, next);
            bytes[at + 4..at + 4 + chunk.len()].copy_from_slice(chunk);
        }
    }
    fn archive() -> Vec<u8> {
        let mut b = vec![0; BLOCK * 7];
        put(&mut b, 0x12c, 0x5442);
        put(&mut b, 0x130, BLOCK as u32);
        put(&mut b, 0x14c, BLOCK as u32);
        let mut n = vec![0; NODE_SIZE];
        put(&mut n, 248, 1);
        put(&mut n, 252, 42);
        put(&mut n, 256, (BLOCK * 4) as u32);
        put(&mut n, 260, 700);
        put(&mut n, 264, 123);
        chain(&mut b, BLOCK, &n);
        chain(
            &mut b,
            BLOCK * 4,
            &(0..700).map(|i| (i % 251) as u8).collect::<Vec<_>>(),
        );
        b
    }
    #[test]
    fn reads_multiblock_node_and_record() {
        let mut bytes = archive();
        // A native leaf ignores unused child words after child[0].
        put(&mut bytes, BLOCK + 4 + 4, 0xdeadbeef);
        let f = Fixture::write(&bytes);
        let mut d = Dat::open(&f.0).unwrap();
        assert_eq!(d.block_size(), 512);
        assert_eq!(d.find(42).unwrap().unwrap().stamp, 123);
        assert_eq!(
            d.read(42).unwrap(),
            (0..700).map(|i| (i % 251) as u8).collect::<Vec<_>>()
        );
        assert_eq!(d.read(43).unwrap_err().kind(), io::ErrorKind::NotFound);
    }
    #[test]
    fn rejects_cyclic_outside_and_truncated_chains() {
        let mut b = archive();
        put(&mut b, BLOCK * 4, (BLOCK * 4) as u32);
        let f = Fixture::write(&b);
        assert!(
            Dat::open(&f.0)
                .unwrap()
                .read(42)
                .unwrap_err()
                .to_string()
                .contains("cyclic")
        );
        let mut b = archive();
        put(&mut b, BLOCK * 4, 0xfffffe00);
        let f = Fixture::write(&b);
        assert!(
            Dat::open(&f.0)
                .unwrap()
                .read(42)
                .unwrap_err()
                .to_string()
                .contains("outside")
        );
        let mut b = archive();
        b.truncate(BLOCK * 5 + 30);
        let f = Fixture::write(&b);
        assert!(Dat::open(&f.0).unwrap().read(42).is_err());
        let f = Fixture::write(&[0; 0x150]);
        assert!(Dat::open(&f.0).is_err());
        let mut b = archive();
        put(&mut b, BLOCK, BLOCK as u32);
        let f = Fixture::write(&b);
        assert!(Dat::open(&f.0).unwrap().find(42).is_err());
    }
    #[test]
    fn directed_tree_lookup_and_tree_cycle() {
        let mut b = archive();
        b.resize(BLOCK * 13, 0);
        let mut root = vec![0; NODE_SIZE];
        put(&mut root, 0, (BLOCK * 7) as u32);
        put(&mut root, 4, (BLOCK * 10) as u32);
        put(&mut root, 248, 1);
        put(&mut root, 252, 50);
        chain(&mut b, BLOCK, &root);
        let mut leaf = vec![0; NODE_SIZE];
        put(&mut leaf, 248, 1);
        put(&mut leaf, 252, 42);
        put(&mut leaf, 256, (BLOCK * 4) as u32);
        put(&mut leaf, 260, 700);
        chain(&mut b, BLOCK * 7, &leaf);
        chain(&mut b, BLOCK * 10, &vec![0; NODE_SIZE]);
        let f = Fixture::write(&b);
        let mut d = Dat::open(&f.0).unwrap();
        assert_eq!(d.read(42).unwrap().len(), 700);
        assert!(d.find(60).unwrap().is_none());
        put(&mut root, 0, BLOCK as u32);
        chain(&mut b, BLOCK, &root);
        let f = Fixture::write(&b);
        assert!(
            Dat::open(&f.0)
                .unwrap()
                .find(42)
                .unwrap_err()
                .to_string()
                .contains("cyclic")
        );
    }
    #[test]
    fn invalid_sizes_counts_sorting_and_runtime_truncation() {
        let mut b = archive();
        put(&mut b, BLOCK + 4 + 248, 62);
        let f = Fixture::write(&b);
        assert!(Dat::open(&f.0).unwrap().read(42).is_err());
        let mut b = archive();
        put(&mut b, BLOCK + 4 + 260, (MAX_RECORD_BYTES + 1) as u32);
        let f = Fixture::write(&b);
        assert!(Dat::open(&f.0).unwrap().read(42).is_err());
        let mut b = archive();
        put(&mut b, 0x130, 1);
        let f = Fixture::write(&b);
        assert!(Dat::open(&f.0).is_err());
        let mut b = archive();
        put(&mut b, BLOCK + 4 + 248, 2);
        put(&mut b, BLOCK + 4 + 268, 41);
        let f = Fixture::write(&b);
        assert!(Dat::open(&f.0).unwrap().read(42).is_err());
        let f = Fixture::write(&archive());
        let mut d = Dat::open(&f.0).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&f.0)
            .unwrap()
            .set_len(BLOCK as u64 + 10)
            .unwrap();
        assert_eq!(d.read(42).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    }
}
