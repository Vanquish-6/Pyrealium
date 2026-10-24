//! Imported route data for explicit development instant portal travel.
use crate::{
    content::Content,
    dat::{self, Dat},
    movement::Placement,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::Path,
};
fn invalid() -> io::Error {
    io::Error::other("unsupported portal route manifest")
}
#[derive(Clone, Copy)]
pub struct Route {
    pub destination: Placement,
    pub level: u16,
}
#[derive(Default)]
pub struct Routes(BTreeMap<u64, Route>);
impl Routes {
    pub fn bundled(content: &Content, game_data: &Path) -> io::Result<Self> {
        let mut lines = include_str!("../content/portal-routes.tsv").lines();
        if lines.next() != Some("pyrealium-portal-routes-v1") {
            return Err(invalid());
        }
        let mut routes = BTreeMap::new();
        let mut seen = BTreeSet::new();
        let mut checked = BTreeSet::new();
        let mut cells = Dat::open(game_data.join("cell_1.dat"))?;
        for line in lines {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.len() > 512 {
                return Err(invalid());
            }
            let f: Vec<_> = line.split('\t').collect();
            if f.len() != 12 {
                return Err(invalid());
            }
            let word = |index: usize| u32::from_str_radix(f[index], 16).map_err(|_| invalid());
            let coordinate = |index: usize| f[index].parse::<f32>().map_err(|_| invalid());
            let id = u64::from_str_radix(f[0], 16).map_err(|_| invalid())?;
            let entity = word(1)?;
            let source = Placement {
                cell: word(2)?,
                coordinates: [coordinate(3)?, coordinate(4)?, coordinate(5)?],
                heading: 0,
            };
            let destination = Placement {
                cell: word(6)?,
                coordinates: [coordinate(7)?, coordinate(8)?, coordinate(9)?],
                heading: f[10].parse().map_err(|_| invalid())?,
            };
            let level = f[11].parse::<u16>().map_err(|_| invalid())?;
            if id >> 48 != 0x8140
                || id == crate::content::NAMESPACE
                || !matches!(entity, 0x4700084d | 0x47001451 | 0x47001454 | 0x4700369a)
                || !source.valid()
                || !destination.valid()
                || !(1..=150).contains(&level)
                || seen.len() >= 1024
                || !seen.insert(id)
            {
                return Err(invalid());
            }
            // The same route bundle supports the base, observed and training
            // content packs. An absent source does not enable a phantom portal.
            let Some(object) = content.object(id) else {
                continue;
            };
            if object.entity != entity
                || object.placement.cell != source.cell
                || object
                    .placement
                    .coordinates
                    .into_iter()
                    .zip(source.coordinates)
                    .any(|(a, b)| (a - b).abs() > 0.01)
            {
                return Err(io::Error::other(
                    "portal route differs from loaded source placement",
                ));
            }
            let record = dat::world_cell_record(destination.cell)?;
            if checked.insert(record) {
                cells.read(record)?;
            }
            routes.insert(id, Route { destination, level });
        }
        Ok(Self(routes))
    }
    pub fn get(&self, id: u64) -> Option<Route> {
        self.0.get(&id).copied()
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
