//! Imported route data for explicit development instant portal travel.
use crate::{
    content::Content,
    dat::{self, Dat},
    movement::Placement,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{self, Read},
    path::Path,
};
fn invalid() -> io::Error {
    io::Error::other("unsupported portal route manifest")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    struct Fixture(std::path::PathBuf);
    fn dat(path: &Path, ids: &[u32]) {
        let mut bytes = vec![0u8; (4 + ids.len()) * 512];
        let put = |data: &mut [u8], at: usize, value: u32| {
            data[at..at + 4].copy_from_slice(&value.to_le_bytes())
        };
        put(&mut bytes, 300, 0x5442);
        put(&mut bytes, 304, 512);
        put(&mut bytes, 332, 512);
        let mut node = vec![0u8; 0x4cc];
        put(&mut node, 248, ids.len() as u32);
        let mut ids = ids.to_vec();
        ids.sort_unstable();
        for (index, id) in ids.iter().enumerate() {
            put(&mut node, 252 + 16 * index, *id);
            put(&mut node, 256 + 16 * index, ((4 + index) * 512) as u32);
            put(&mut node, 260 + 16 * index, 1);
            bytes[(4 + index) * 512 + 4] = 1;
        }
        for (index, chunk) in node.chunks(508).enumerate() {
            let at = (index + 1) * 512;
            put(
                &mut bytes,
                at,
                if index < 2 { (at + 512) as u32 } else { 0 },
            );
            bytes[at + 4..at + 4 + chunk.len()].copy_from_slice(chunk);
        }
        std::fs::write(path, bytes).unwrap();
    }
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "ac2-entrances-{}-{}",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            dat(&path.join("portal.dat"), &[0x47001454, 0x1f000d09]);
            dat(
                &path.join("cell_1.dat"),
                &[0x74baffff, 0x02990109, 0x0299010a],
            );
            std::fs::write(path.join("world.tsv"),"pyrealium-world-v2\n8140000000030000\t47001454\t1f000d09\t74ba0007\t8\t12\t2\t1\t0\t0\t0\n8140000000030001\t47001454\t1f000d09\t02990109\t50\t220\t-44\t1\t0\t0\t0\n").unwrap();
            Self(path)
        }
        fn content(&self) -> Content {
            Content::load(&self.0.join("world.tsv"), &self.0).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    const PAIR: &str = "pyrealium-portal-routes-v1\n8140000000030000\t47001454\t74ba0007\t8\t12\t2\t0299010a\t10\t20\t-40\t191\t5\n8140000000030001\t47001454\t02990109\t50\t220\t-44\t74ba0008\t35\t50\t2\t64\t1\n";
    #[test]
    fn external_pair_routes_to_new_room_and_return_with_heading() {
        let fixture = Fixture::new();
        std::fs::write(fixture.0.join("world.routes.tsv"), PAIR).unwrap();
        let routes = Routes::for_content(
            &fixture.content(),
            &fixture.0,
            Some(&fixture.0.join("world.tsv")),
        )
        .unwrap();
        assert_eq!(routes.len(), 2);
        let enter = routes.get(0x8140000000030000).unwrap();
        assert_eq!(
            enter.destination,
            Placement {
                cell: 0x0299010a,
                coordinates: [10.0, 20.0, -40.0],
                heading: 191
            }
        );
        assert_eq!(enter.level, 5);
        let leave = routes.get(0x8140000000030001).unwrap();
        assert_eq!(leave.destination.cell, 0x74ba0008);
        assert_eq!(leave.destination.heading, 64);
    }
    #[test]
    fn no_sidecar_keeps_defaults_and_never_adds_phantom_routes() {
        let fixture = Fixture::new();
        assert!(
            Routes::for_content(
                &fixture.content(),
                &fixture.0,
                Some(&fixture.0.join("world.tsv"))
            )
            .unwrap()
            .is_empty()
        );
        assert!(
            Routes::bundled(&fixture.content(), &fixture.0)
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn authored_pair_preserves_retained_routes_and_cannot_override_them() {
        let fixture = Fixture::new();
        dat(
            &fixture.0.join("cell_1.dat"),
            &[0x74baffff, 0x02990109, 0x0299010a, 0x6fa7ffff, 0x849fffff],
        );
        let path = fixture.0.join("world.tsv");
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("8140000000010001\t47001454\t1f000d09\t6fa7001b\t65.0608\t55.4285\t79.9443\t1\t0\t0\t0\n");
        std::fs::write(&path, text).unwrap();
        let content = fixture.content();
        assert_eq!(Routes::bundled(&content, &fixture.0).unwrap().len(), 1);
        std::fs::write(fixture.0.join("world.routes.tsv"), PAIR).unwrap();
        let routes = Routes::for_content(&content, &fixture.0, Some(&path)).unwrap();
        assert_eq!(routes.len(), 3);
        assert_eq!(
            routes.get(0x8140000000010001).unwrap().destination.cell,
            0x849f0026
        );
        let duplicate = "pyrealium-portal-routes-v1\n8140000000010001\t47001454\t6fa7001b\t65.0608\t55.4285\t79.9443\t0299010a\t10\t20\t-40\t191\t1\n";
        std::fs::write(fixture.0.join("world.routes.tsv"), duplicate).unwrap();
        assert!(Routes::for_content(&content, &fixture.0, Some(&path)).is_err());
    }
    #[test]
    fn external_route_rejects_missing_or_mismatched_sources_and_destination() {
        let fixture = Fixture::new();
        let content = fixture.content();
        for text in [
            PAIR.replace("8140000000030000", "8140000000030099"),
            PAIR.replace("\t8\t12\t2\t", "\t9\t12\t2\t"),
            PAIR.replace("0299010a", "0299010b"),
            PAIR.replace("0299010a", "0299ffff"),
            PAIR.replace("\t191\t5", "\t256\t5"),
            PAIR.replace("\t191\t5", "\t191\t0"),
            PAIR.replace("\t10\t20\t-40", "\tNaN\t20\t-40"),
        ] {
            assert!(
                Routes::parse(&text, &content, &fixture.0, true).is_err(),
                "accepted {text}"
            );
        }
    }
    #[test]
    fn external_route_bounds_duplicates_and_utf8_are_checked() {
        let fixture = Fixture::new();
        let content = fixture.content();
        let duplicate = format!("{PAIR}{}\n", PAIR.lines().nth(1).unwrap());
        assert!(Routes::parse(&duplicate, &content, &fixture.0, true).is_err());
        assert!(Routes::parse(&"x".repeat(512 * 1024 + 1), &content, &fixture.0, true).is_err());
        std::fs::write(fixture.0.join("world.routes.tsv"), [0xff]).unwrap();
        assert!(
            Routes::for_content(&content, &fixture.0, Some(&fixture.0.join("world.tsv"))).is_err()
        );
    }
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
        Self::parse(
            include_str!("../content/portal-routes.tsv"),
            content,
            game_data,
            false,
        )
    }
    /// An authored world.tsv may carry world.routes.tsv beside it. External
    /// routes extend the retained defaults, and never silently replace them.
    pub fn for_content(
        content: &Content,
        game_data: &Path,
        path: Option<&Path>,
    ) -> io::Result<Self> {
        let mut routes = Self::bundled(content, game_data)?;
        if let Some(path) =
            path.filter(|p| !matches!(p.to_str(), Some("none" | "observed" | "training")))
        {
            let sidecar = path.with_extension("routes.tsv");
            let mut bytes = Vec::new();
            let file = match File::open(&sidecar) {
                Ok(file) => Some(file),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            };
            if let Some(file) = file {
                file.take(512 * 1024 + 1).read_to_end(&mut bytes)?;
                if bytes.len() > 512 * 1024 {
                    return Err(invalid());
                }
                let text = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
                let authored = Self::parse(text, content, game_data, true)?;
                for (id, route) in authored.0 {
                    if routes.0.insert(id, route).is_some() {
                        return Err(io::Error::other(
                            "authored route collides with a retained portal route",
                        ));
                    }
                }
                if routes.len() > 2048 {
                    return Err(invalid());
                }
            }
        }
        Ok(routes)
    }
    fn parse(text: &str, content: &Content, game_data: &Path, strict: bool) -> io::Result<Self> {
        if text.len() > 512 * 1024 {
            return Err(invalid());
        }
        let mut lines = text.lines();
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
                if strict {
                    return Err(io::Error::other(
                        "authored portal has no loaded source object",
                    ));
                }
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
            for cell in [source.cell, destination.cell] {
                let record = dat::world_cell_record(cell)?;
                if checked.insert(record) {
                    cells.read(record)?;
                }
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
