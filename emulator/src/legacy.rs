//! Read-only extracted terrain records, checked against recovered build-7148
//! CLandBlockData::StreamPack (0048f310), PSmartArray serializers (0048faa0,
//! 006388c0), and DBObj::UnPack_Type (006d7830). See docs/world-evidence.md.
//! Corpus validation does not establish client content or gameplay compatibility.
use std::{
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

const MAX_RECORD_BYTES: u64 = 1024 * 1024;
const SIDE: usize = 17;
const SAMPLES: usize = SIDE * SIDE;

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Numeric IDs generate the entire relative path; no client string becomes a path.
pub fn record_path(root: &Path, id: u32) -> PathBuf {
    root.join(format!("0x{:02X}", id >> 24))
        .join(format!("0x{id:08X}"))
}

/// Bound the read even if the file grows after metadata was obtained.
pub fn read_record(root: &Path, id: u32) -> io::Result<Vec<u8>> {
    let file = File::open(record_path(root, id))?;
    let size = file.metadata()?.len();
    if size == 0 || size > MAX_RECORD_BYTES {
        return Err(invalid("empty or oversized legacy record"));
    }
    let mut bytes = Vec::with_capacity(size as usize);
    file.take(MAX_RECORD_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(invalid("empty or oversized legacy record"));
    }
    Ok(bytes)
}

/// Borrowed, allocation-free terrain view. Terrain words are decoded only on use.
#[derive(Debug)]
pub struct Terrain<'a> {
    pub id: u32,
    pub info_id: u32,
    pub pathmap_id: u32,
    heights: &'a [u8],
    terrain: &'a [u8],
}

impl<'a> Terrain<'a> {
    pub fn decode(bytes: &'a [u8], expected_id: u32) -> io::Result<Self> {
        // 3 header words, a height count, height bytes, terrain count/words.
        const END: usize = 16 + SAMPLES + 4 + SAMPLES * 4;
        if bytes.len() != END && bytes.len() != END.next_multiple_of(4) {
            return Err(invalid("unsupported legacy terrain record length"));
        }
        let word = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let id = word(0);
        let info_id = word(4);
        // Some blocks in the supplied corpus use zero for no associated info.
        if id != expected_id || id & 0xffff != 0xffff || (info_id != 0 && info_id != id - 1) {
            return Err(invalid("legacy terrain identity or info link mismatch"));
        }
        if word(12) != SAMPLES as u32 || word(16 + SAMPLES) != SAMPLES as u32 {
            return Err(invalid(
                "unsupported legacy terrain grid (expected 17 x 17)",
            ));
        }
        if bytes[END..].iter().any(|b| *b != 0) {
            return Err(invalid("nonzero trailing legacy terrain bytes"));
        }
        Ok(Self {
            id,
            info_id,
            pathmap_id: word(8),
            heights: &bytes[16..16 + SAMPLES],
            terrain: &bytes[20 + SAMPLES..END],
        })
    }

    /// Raw height byte, not world-space elevation. The height table is separate.
    pub fn height(&self, x: usize, y: usize) -> Option<u8> {
        if x >= SIDE || y >= SIDE {
            return None;
        }
        Some(self.heights[y * SIDE + x])
    }

    pub fn terrain_word(&self, x: usize, y: usize) -> Option<u32> {
        if x >= SIDE || y >= SIDE {
            return None;
        }
        let start = (y * SIDE + x) * 4;
        Some(u32::from_le_bytes(
            self.terrain[start..start + 4].try_into().unwrap(),
        ))
    }

    pub fn height_range(&self) -> (u8, u8) {
        (
            *self.heights.iter().min().unwrap(),
            *self.heights.iter().max().unwrap(),
        )
    }
}

#[derive(Debug, Default)]
pub struct Validation {
    pub terrain_records: usize,
    pub missing_info_records: usize,
}

/// Validate every terrain file in canonical numeric buckets, reading one at a time.
/// Does not parse info/envcell records or turn this corpus into authoritative physics.
pub fn validate_terrain(root: &Path) -> io::Result<Validation> {
    let mut result = Validation::default();
    for bucket in 0..=255 {
        let path = root.join(format!("0x{bucket:02X}"));
        let entries = match std::fs::read_dir(path) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name.len() != 10 || !name.starts_with("0x") {
                continue;
            }
            let Ok(id) = u32::from_str_radix(&name[2..], 16) else {
                continue;
            };
            if id & 0xffff != 0xffff {
                continue;
            }
            if id >> 24 != bucket {
                return Err(invalid("terrain file in wrong bucket"));
            }
            let bytes = read_record(root, id)?;
            let terrain =
                Terrain::decode(&bytes, id).map_err(|e| invalid(&format!("{name}: {e}")))?;
            result.terrain_records += 1;
            if terrain.info_id == 0 {
                continue;
            }
            match std::fs::metadata(record_path(root, terrain.info_id)) {
                Ok(m) if m.is_file() && m.len() > 0 => {}
                Ok(_) => result.missing_info_records += 1,
                Err(e) if e.kind() == io::ErrorKind::NotFound => result.missing_info_records += 1,
                Err(e) => return Err(e),
            }
        }
    }
    if result.terrain_records == 0 {
        return Err(invalid("no legacy terrain records found"));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Vec<u8> {
        let mut bytes = Vec::new();
        for word in [0x0299ffffu32, 0x0299fffe, 7, SAMPLES as u32] {
            bytes.extend(word.to_le_bytes());
        }
        bytes.extend((0..SAMPLES).map(|i| (i % 256) as u8));
        bytes.extend((SAMPLES as u32).to_le_bytes());
        for i in 0..SAMPLES {
            bytes.extend((i as u32 * 123).to_le_bytes());
        }
        bytes
    }
    #[test]
    fn recovered_layout_and_grid_edges() {
        let bytes = fixture();
        let terrain = Terrain::decode(&bytes, 0x0299ffff).unwrap();
        assert_eq!(terrain.pathmap_id, 7);
        assert_eq!(terrain.height(16, 16), Some(32));
        assert_eq!(terrain.terrain_word(16, 16), Some(288 * 123));
        assert_eq!(terrain.height_range(), (0, 255));
        for (x, y) in [(17, 0), (0, 17), (usize::MAX, usize::MAX)] {
            assert_eq!(terrain.height(x, y), None);
            assert_eq!(terrain.terrain_word(x, y), None);
        }
        assert_eq!(
            record_path(Path::new("world"), 0x0299ffff),
            Path::new("world/0x02/0x0299FFFF")
        );
    }
    #[test]
    fn rejects_truncations_counts_identity_and_extra_data() {
        let bytes = fixture();
        for end in 0..bytes.len() {
            assert!(Terrain::decode(&bytes[..end], 0x0299ffff).is_err());
        }
        for offset in [0, 4, 12, 16 + SAMPLES] {
            let mut broken = bytes.clone();
            broken[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            assert!(Terrain::decode(&broken, 0x0299ffff).is_err());
        }
        let mut padded = bytes;
        for padding in [1, 2, 4] {
            let mut malformed = padded.clone();
            malformed.extend(vec![0; padding]);
            assert!(Terrain::decode(&malformed, 0x0299ffff).is_err());
        }
        padded.extend([0, 0, 0]);
        Terrain::decode(&padded, 0x0299ffff).unwrap();
        *padded.last_mut().unwrap() = 1;
        assert!(Terrain::decode(&padded, 0x0299ffff).is_err());
        assert!(Terrain::decode(&fixture(), 0x0298ffff).is_err());
        let mut blank = fixture();
        blank[4..8].fill(0);
        assert_eq!(Terrain::decode(&blank, 0x0299ffff).unwrap().info_id, 0);
    }
    #[test]
    fn file_reader_limits_and_link_validation() {
        let root = std::env::temp_dir().join(format!(
            "pyrealium-legacy-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("0x02")).unwrap();
        assert!(validate_terrain(&root).is_err());
        let path = record_path(&root, 0x0299ffff);
        std::fs::write(&path, fixture()).unwrap();
        assert_eq!(read_record(&root, 0x0299ffff).unwrap(), fixture());
        let missing = validate_terrain(&root).unwrap();
        assert_eq!(
            (missing.terrain_records, missing.missing_info_records),
            (1, 1)
        );
        std::fs::write(record_path(&root, 0x0299fffe), [1]).unwrap();
        assert_eq!(validate_terrain(&root).unwrap().missing_info_records, 0);
        std::fs::write(&path, []).unwrap();
        assert!(read_record(&root, 0x0299ffff).is_err());
        File::create(&path)
            .unwrap()
            .set_len(MAX_RECORD_BYTES + 1)
            .unwrap();
        assert!(read_record(&root, 0x0299ffff).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    #[ignore = "requires AC2_TEST_LANDBLOCKS pointing to the supplied extracted LandBlocks directory"]
    fn supplied_world_terrain_corpus() {
        let root = std::env::var_os("AC2_TEST_LANDBLOCKS").expect("AC2_TEST_LANDBLOCKS");
        let result = validate_terrain(Path::new(&root)).unwrap();
        assert!(result.terrain_records > 0);
        println!("{result:?}");
    }
}
