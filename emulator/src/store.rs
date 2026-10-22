//! Append-only local character journal. A write is acknowledged only after fsync.
//! Corruption/torn tails stop startup rather than silently discarding characters.
use crate::world::CharacterCreationRequest;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const MAX_JOURNAL_BYTES: u64 = 16 * 1024 * 1024;
const MAX_PHYSICAL_VALUES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Character {
    pub id: u64,
    pub account: String,
    pub name: String,
    /// Exact ordered client appearance values; None for legacy CLI-created characters.
    pub creation: Option<CharacterCreationRequest>,
}

pub struct Store {
    file: File,
    lock: PathBuf,
    pub characters: BTreeMap<u64, Character>,
    poisoned: bool,
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}
fn hash(bytes: &[u8]) -> u32 {
    bytes.iter().fold(2166136261u32, |h, b| {
        (h ^ u32::from(*b)).wrapping_mul(16777619)
    })
}
fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b' ' || b == b'-')
}

impl Store {
    pub fn open(dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let lock = dir.join("server.lock");
        let mut guard = OpenOptions::new().write(true).create_new(true).open(&lock)
            .map_err(|e| io::Error::new(e.kind(), format!("cannot lock {}: {e}; after a crash, verify no server is running before removing this lock", lock.display())))?;
        let result = (|| {
            writeln!(guard, "{}", std::process::id())?;
            guard.sync_all()?;
            let mut file = OpenOptions::new()
                .create(true)
                .read(true)
                .append(true)
                .open(dir.join("characters.journal"))?;
            if file.metadata()?.len() > MAX_JOURNAL_BYTES {
                return Err(invalid("journal exceeds 16 MiB limit"));
            }
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            let characters = decode_journal(&bytes)?;
            Ok(Self {
                file,
                lock: lock.clone(),
                characters,
                poisoned: false,
            })
        })();
        if result.is_err() {
            drop(guard);
            let _ = fs::remove_file(lock);
        }
        result
    }
    pub fn create(&mut self, account: &str, name: &str) -> io::Result<Character> {
        self.create_record(account, name, None)
    }
    /// Persists structurally valid input. The caller must validate entity/world authority.
    pub fn create_native(
        &mut self,
        account: &str,
        request: &CharacterCreationRequest,
    ) -> io::Result<Character> {
        self.create_record(account, &request.name, Some(request))
    }
    fn create_record(
        &mut self,
        account: &str,
        name: &str,
        creation: Option<&CharacterCreationRequest>,
    ) -> io::Result<Character> {
        if self.poisoned {
            return Err(invalid(
                "journal write failed previously; restart and inspect journal",
            ));
        }
        if !valid_name(account) || !valid_name(name) {
            return Err(invalid(
                "account and character names must contain 1-32 ASCII letters, digits, spaces, hyphens or underscores",
            ));
        }
        if self
            .characters
            .values()
            .any(|c| c.name.eq_ignore_ascii_case(name))
        {
            return Err(invalid("character name already exists"));
        }
        if self
            .characters
            .values()
            .filter(|c| c.account == account)
            .count()
            >= 5
        {
            return Err(invalid("local character limit is five"));
        }
        let id = self
            .characters
            .keys()
            .next_back()
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| invalid("character IDs exhausted"))?;
        let c = Character {
            id,
            account: account.into(),
            name: name.into(),
            creation: creation.cloned(),
        };
        let line = match creation {
            None => format!("v1\t{}\t{}\t{}", c.id, c.account, c.name),
            Some(request) => format!(
                "v2\t{}\t{}\t{}\t{}",
                c.id,
                c.account,
                c.name,
                encode_creation(request)?
            ),
        };
        let record = format!("{line}\t{:08x}\n", hash(line.as_bytes()));
        if self
            .file
            .metadata()?
            .len()
            .saturating_add(record.len() as u64)
            > MAX_JOURNAL_BYTES
        {
            return Err(invalid("journal exceeds 16 MiB limit"));
        }
        if let Err(e) = self
            .file
            .write_all(record.as_bytes())
            .and_then(|()| self.file.sync_all())
        {
            self.poisoned = true;
            return Err(e);
        }
        self.characters.insert(id, c.clone());
        Ok(c)
    }
}
impl Drop for Store {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.lock);
    }
}

pub fn decode_journal(bytes: &[u8]) -> io::Result<BTreeMap<u64, Character>> {
    if bytes.len() as u64 > MAX_JOURNAL_BYTES {
        return Err(invalid("journal exceeds 16 MiB limit"));
    }
    if !bytes.is_empty() && bytes.last() != Some(&b'\n') {
        return Err(invalid("incomplete character journal tail"));
    }
    let text =
        std::str::from_utf8(bytes).map_err(|_| invalid("invalid character journal encoding"))?;
    let mut records = BTreeMap::new();
    for line in text.lines() {
        let (data, sum) = line
            .rsplit_once('\t')
            .ok_or_else(|| invalid("missing journal checksum"))?;
        if u32::from_str_radix(sum, 16).ok() != Some(hash(data.as_bytes())) {
            return Err(invalid("character journal checksum mismatch"));
        }
        let fields: Vec<_> = data.split('\t').collect();
        if !((fields.len() == 4 && fields[0] == "v1") || (fields.len() == 5 && fields[0] == "v2"))
            || !valid_name(fields[2])
            || !valid_name(fields[3])
        {
            return Err(invalid("invalid journal record"));
        }
        let id = fields[1]
            .parse::<u64>()
            .map_err(|_| invalid("invalid character ID"))?;
        if id == 0 || records.contains_key(&id) {
            return Err(invalid("duplicate or zero character ID"));
        }
        if records
            .values()
            .any(|c: &Character| c.name.eq_ignore_ascii_case(fields[3]))
        {
            return Err(invalid("duplicate character name in journal"));
        }
        if records.values().filter(|c| c.account == fields[2]).count() >= 5 {
            return Err(invalid("account exceeds local character limit"));
        }
        let creation = if fields[0] == "v2" {
            Some(decode_creation(fields[3], fields[4])?)
        } else {
            None
        };
        records.insert(
            id,
            Character {
                id,
                account: fields[2].into(),
                name: fields[3].into(),
                creation,
            },
        );
    }
    Ok(records)
}

// Journal v2 uses canonical little-endian words encoded as hex, independent of network
// string padding. Physical tuple order and float bits survive without normalization.
fn encode_creation(request: &CharacterCreationRequest) -> io::Result<String> {
    if request.physical_values.len() > MAX_PHYSICAL_VALUES {
        return Err(invalid("too many physical values"));
    }
    let mut keys = std::collections::BTreeSet::new();
    if request
        .physical_values
        .iter()
        .any(|(key, bits)| !keys.insert(*key) || !f32::from_bits(*bits).is_finite())
    {
        return Err(invalid("invalid or duplicate physical value"));
    }
    let mut bytes = Vec::with_capacity(28 + request.physical_values.len() * 8);
    for word in [
        request.entity_did,
        request.start_area,
        request.race,
        request.sex,
        request.class,
        request.implement,
        request.physical_values.len() as u32,
    ] {
        bytes.extend(word.to_le_bytes());
    }
    for (key, bits) in &request.physical_values {
        bytes.extend(key.to_le_bytes());
        bytes.extend(bits.to_le_bytes());
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 15) as usize] as char);
    }
    Ok(out)
}
fn decode_creation(name: &str, hex: &str) -> io::Result<CharacterCreationRequest> {
    if hex.len() < 56 || hex.len() > (28 + MAX_PHYSICAL_VALUES * 8) * 2 || hex.len() % 8 != 0 {
        return Err(invalid("invalid native character data length"));
    }
    fn nibble(b: u8) -> io::Result<u8> {
        match b {
            b'0'..=b'9' => Ok(b - b'0'),
            b'a'..=b'f' => Ok(b - b'a' + 10),
            _ => Err(invalid("invalid native character hex")),
        }
    }
    let bytes: Vec<u8> = hex
        .as_bytes()
        .chunks_exact(2)
        .map(|p| Ok((nibble(p[0])? << 4) | nibble(p[1])?))
        .collect::<io::Result<_>>()?;
    let words: Vec<u32> = bytes
        .chunks_exact(4)
        .map(|w| u32::from_le_bytes(w.try_into().unwrap()))
        .collect();
    let count = words[6] as usize;
    if count > MAX_PHYSICAL_VALUES || words.len() != 7 + count * 2 {
        return Err(invalid("invalid native character physical count"));
    }
    let request = CharacterCreationRequest {
        name: name.into(),
        entity_did: words[0],
        start_area: words[1],
        race: words[2],
        sex: words[3],
        class: words[4],
        implement: words[5],
        physical_values: words[7..].chunks_exact(2).map(|w| (w[0], w[1])).collect(),
    };
    encode_creation(&request)?;
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(name: &str) -> CharacterCreationRequest {
        CharacterCreationRequest {
            name: name.into(),
            entity_did: 0x10001234,
            start_area: 7,
            race: 2,
            sex: 1,
            class: 3,
            implement: 4,
            physical_values: vec![(9, (-0.0f32).to_bits()), (2, 0.1234567f32.to_bits())],
        }
    }
    fn record(data: &str) -> Vec<u8> {
        format!("{data}\t{:08x}\n", hash(data.as_bytes())).into_bytes()
    }
    #[test]
    fn mixed_versions_retain_exact_native_appearance_after_restart() {
        let dir = std::env::temp_dir().join(
            format!(
                "ac2-native-journal-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            )
            .replace(['(', ')'], ""),
        );
        let original = request("Native Hero");
        {
            let mut store = Store::open(&dir).unwrap();
            assert_eq!(store.create("local", "Legacy Hero").unwrap().creation, None);
            let created = store.create_native("local", &original).unwrap();
            assert_eq!(created.creation.as_ref(), Some(&original));
            assert!(
                store
                    .create_native("other", &request("native hero"))
                    .is_err()
            );
            for i in 0..3 {
                store
                    .create_native("local", &request(&format!("Extra {i}")))
                    .unwrap();
            }
            assert!(store.create_native("local", &request("Sixth")).is_err());
        }
        {
            let store = Store::open(&dir).unwrap();
            assert_eq!(store.characters.len(), 5);
            assert_eq!(store.characters[&1].creation, None);
            assert_eq!(store.characters[&2].creation.as_ref(), Some(&original));
        }
        fs::remove_file(dir.join("characters.journal")).unwrap();
        fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn native_records_reject_torn_corrupt_and_semantically_malformed_data() {
        let req = request("Hero");
        let encoded = encode_creation(&req).unwrap();
        let good = record(&format!("v2\t1\tlocal\tHero\t{encoded}"));
        assert_eq!(
            decode_journal(&good).unwrap()[&1].creation.as_ref(),
            Some(&req)
        );
        for n in 1..good.len() {
            assert!(
                decode_journal(&good[..n]).is_err(),
                "accepted torn prefix {n}"
            );
        }
        for n in 0..encoded.len() {
            assert!(
                decode_journal(&record(&format!("v2\t1\tlocal\tHero\t{}", &encoded[..n]))).is_err()
            );
        }
        let mut malformed = encoded.clone();
        malformed.replace_range(48..56, "ffffffff");
        assert!(decode_journal(&record(&format!("v2\t1\tlocal\tHero\t{malformed}"))).is_err());
        let mut invalid = req.clone();
        invalid.physical_values.push(invalid.physical_values[0]);
        assert!(encode_creation(&invalid).is_err());
        invalid.physical_values = vec![(1, f32::NAN.to_bits())];
        assert!(encode_creation(&invalid).is_err());
        invalid.physical_values = (0..257).map(|i| (i, 0)).collect();
        assert!(encode_creation(&invalid).is_err());
        let mut mixed = record("v1\t1\tlocal\tHero");
        mixed.extend(record("v1\t2\tother\thero"));
        assert!(decode_journal(&mixed).is_err());
        let mut too_many = Vec::new();
        for i in 1..=6 {
            too_many.extend(record(&format!("v1\t{i}\tlocal\tHero {i}")));
        }
        assert!(decode_journal(&too_many).is_err());
    }
    #[test]
    fn persistence_exclusive_writer_and_corruption() {
        let dir = std::env::temp_dir().join(
            format!(
                "ac2-journal-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            )
            .replace(['(', ')'], ""),
        );
        fs::create_dir_all(&dir).unwrap();
        {
            let mut s = Store::open(&dir).unwrap();
            assert!(Store::open(&dir).is_err());
            assert_eq!(s.create("local", "Explorer").unwrap().id, 1);
            assert!(s.create("local", "explorer").is_err());
        }
        {
            let s = Store::open(&dir).unwrap();
            assert_eq!(s.characters[&1].name, "Explorer");
        }
        let path = dir.join("characters.journal");
        let mut bytes = fs::read(&path).unwrap();
        bytes[0] ^= 1;
        assert!(decode_journal(&bytes).is_err());
        assert!(decode_journal(&bytes[..bytes.len() - 1]).is_err());
        fs::remove_file(path).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
