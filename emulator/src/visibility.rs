//! Read-only native interior potentially-visible-cell sets.
//!
//! CEnvCell::StreamPack 00482da0 serializes portals, visible cells, then shared
//! cells. CEnvCell::InqVisibleCells 00481630, GrabVisibleCells 00486f10 and
//! GatherVisibleInsideObjects 00485800 consume the first cell array. Shared
//! cells and recursive portal reachability are not substituted for that PVS.
//! This is visibility interest, not a line-of-sight or movement permission test.
use crate::dat::{Dat, world_cell_record};
use std::{collections::BTreeMap, io, path::Path};

const MAX_RECORD: usize = 4 * 1024 * 1024;
const MAX_CELLS: usize = 8192;
const MAX_LINKS: usize = 4096;
const MAX_TOTAL_LINKS: usize = 1_048_576;

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl Reader<'_> {
    fn take(&mut self, size: usize) -> io::Result<&[u8]> {
        let end = self
            .offset
            .checked_add(size)
            .ok_or_else(|| invalid("visibility offset overflow"))?;
        let data = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| invalid("truncated native visibility data"))?;
        self.offset = end;
        Ok(data)
    }
    fn word(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn short(&mut self) -> io::Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn count(&mut self) -> io::Result<usize> {
        let count = self.word()? as usize;
        if count > MAX_LINKS {
            return Err(invalid("native visibility count exceeds bounds"));
        }
        Ok(count)
    }
    fn align(&mut self) -> io::Result<()> {
        self.take((4 - self.offset % 4) % 4)?;
        Ok(())
    }
    fn cells(&mut self, block: u32) -> io::Result<Vec<u32>> {
        let count = self.count()?;
        let mut cells = Vec::with_capacity(count);
        for _ in 0..count {
            let value = block | u32::from(self.short()?);
            world_cell_record(value)?;
            cells.push(value);
        }
        self.align()?;
        Ok(cells)
    }
}

struct Entry {
    visible: Vec<u32>,
    adjacent: Vec<u32>,
    touches_outside: bool,
}
fn decode(cell: u32, bytes: &[u8]) -> io::Result<Entry> {
    if !(44..=MAX_RECORD).contains(&bytes.len()) {
        return Err(invalid("native visibility record size"));
    }
    let mut reader = Reader { bytes, offset: 0 };
    if reader.word()? != cell {
        return Err(invalid("native visibility cell identity mismatch"));
    }
    let mut values = [0f32; 7];
    for value in &mut values {
        *value = f32::from_bits(reader.word()?);
        if !value.is_finite() || value.abs() >= 1_000_000.0 {
            return Err(invalid("invalid native cell frame"));
        }
    }
    if (values[3..].iter().map(|value| value * value).sum::<f32>() - 1.0).abs() > 0.01 {
        return Err(invalid("invalid native cell orientation"));
    }
    if reader.word()? & !0x1f != 0 {
        return Err(invalid("unsupported native cell flags"));
    }
    if reader.word()? >> 24 != 1 {
        return Err(invalid("unsupported native environment mesh"));
    }
    let portals = reader.count()?;
    let block = cell & 0xffff0000;
    let mut adjacent = Vec::new();
    let mut touches_outside = false;
    for _ in 0..portals {
        let flags = reader.short()?;
        if flags & !7 != 0 {
            return Err(invalid("unsupported native cell portal flags"));
        }
        reader.short()?; // Signed portal index; not used as an array index here.
        let other = reader.short()?;
        reader.short()?;
        if flags & 2 != 0 {
            touches_outside = true;
        } else {
            let target = block | u32::from(other);
            world_cell_record(target)?;
            adjacent.push(target);
        }
        if flags & 4 != 0 {
            let count = usize::from(reader.short()?);
            if count > MAX_LINKS {
                return Err(invalid("native portal stab count exceeds bounds"));
            }
            reader.take(count * 2)?;
            reader.align()?;
        }
    }
    reader.align()?;
    let mut visible = reader.cells(block)?;
    // Consume and bound the distinct shared-cell list without promoting it to
    // visibility. Entity/property tails have no role in interest membership.
    reader.cells(block)?;
    visible.push(cell);
    visible.sort_unstable();
    visible.dedup();
    adjacent.sort_unstable();
    adjacent.dedup();
    Ok(Entry {
        visible,
        adjacent,
        touches_outside,
    })
}

pub struct Visibility {
    archive: Dat,
    cache: BTreeMap<u32, Entry>,
    total_links: usize,
}
impl Visibility {
    pub fn open(game_data: &Path) -> io::Result<Self> {
        Ok(Self {
            archive: Dat::open(game_data.join("cell_1.dat"))?,
            cache: BTreeMap::new(),
            total_links: 0,
        })
    }
    /// Prepare on scene activation and before accepting a new observer cell.
    /// Queries stay immutable and do not perform per-recipient DAT reads.
    pub fn prepare(&mut self, cell: u32) -> io::Result<()> {
        if self.cache.contains_key(&cell) {
            return Ok(());
        }
        let backing = world_cell_record(cell)?;
        if self.cache.len() >= MAX_CELLS {
            return Err(invalid("visibility cache cell limit"));
        }
        let entry = if cell & 0xffff < 0x100 {
            // Native outdoor visibility uses LScapeManager near-block settings.
            // Keep only the validated backing identity in the cache; queries
            // apply the explicitly authored outdoor policy below.
            if self.archive.find(backing)?.is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "outdoor visibility backing block missing",
                ));
            }
            Entry {
                visible: vec![cell],
                adjacent: Vec::new(),
                touches_outside: false,
            }
        } else {
            let info = self.archive.find(cell)?.ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "interior visibility cell missing")
            })?;
            if !(44..=MAX_RECORD).contains(&(info.length as usize)) {
                return Err(invalid("native visibility record size"));
            }
            decode(cell, &self.archive.read(cell)?)?
        };
        let size = entry.visible.len() + entry.adjacent.len();
        let total = self
            .total_links
            .checked_add(size)
            .filter(|count| *count <= MAX_TOTAL_LINKS)
            .ok_or_else(|| invalid("visibility cache link limit"))?;
        self.cache.insert(cell, entry);
        self.total_links = total;
        Ok(())
    }
    /// Directed PVS membership. Do not reverse observer and target arguments.
    pub fn visible(&self, observer: u32, target: u32) -> bool {
        observer == target
            || outdoor_near(observer, target)
            || self
                .cache
                .get(&observer)
                .is_some_and(|entry| entry.visible.binary_search(&target).is_ok())
    }
    pub fn cells(&self, cell: u32) -> Vec<u32> {
        if (1..=64).contains(&(cell & 0xffff)) {
            // Explicit development outdoor interest: the surrounding 3x3
            // landblocks. This is not a recovered nearWidth configuration.
            let mut cells = Vec::with_capacity(576);
            let x = (cell >> 24) as i32;
            let y = ((cell >> 16) & 255) as i32;
            for dx in -1..=1 {
                for dy in -1..=1 {
                    if !(0..=255).contains(&(x + dx)) || !(0..=255).contains(&(y + dy)) {
                        continue;
                    }
                    let block = ((x + dx) as u32) << 24 | ((y + dy) as u32) << 16;
                    cells.extend((1..=64).map(|id| block | id));
                }
            }
            return cells;
        }
        self.cache
            .get(&cell)
            .map_or_else(|| vec![cell], |entry| entry.visible.clone())
    }
    pub fn adjacent(&self, cell: u32) -> Vec<u32> {
        self.cache
            .get(&cell)
            .map_or_else(Vec::new, |entry| entry.adjacent.clone())
    }
    pub fn touches_outside(&self, cell: u32) -> bool {
        self.cache
            .get(&cell)
            .is_some_and(|entry| entry.touches_outside)
    }
}

fn outdoor_near(observer: u32, target: u32) -> bool {
    (1..=64).contains(&(observer & 0xffff))
        && (1..=64).contains(&(target & 0xffff))
        && ((observer >> 24) as i32 - (target >> 24) as i32).abs() <= 1
        && (((observer >> 16) & 255) as i32 - ((target >> 16) & 255) as i32).abs() <= 1
}
