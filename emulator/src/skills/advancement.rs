use super::{SkillError, fingerprint};
use crate::dat::Dat;
use std::{collections::BTreeMap, sync::OnceLock};

pub(super) struct Curve {
    fingerprint: u64,
    xp: Vec<u64>,
}
impl Curve {
    pub(super) fn next(&self, spent: u64, cap: u16) -> Result<(u16, u64), SkillError> {
        let level = self.xp.partition_point(|xp| *xp <= spent).saturating_sub(1);
        let cap = usize::from(cap).min(self.xp.len() - 1);
        if level >= cap {
            return Err(SkillError::MaximumRank);
        }
        let next = level + 1;
        if self.xp[next] <= spent {
            return Err(SkillError::Catalog);
        }
        Ok((next as u16, self.xp[next]))
    }
}
fn curves() -> Result<&'static BTreeMap<u32, Curve>, SkillError> {
    static CURVES: OnceLock<Result<BTreeMap<u32, Curve>, SkillError>> = OnceLock::new();
    CURVES
        .get_or_init(|| {
            let mut lines = include_str!("../../content/skill-advancement.tsv").lines();
            if lines.next() != Some("pyrealium-skill-advancement-v1") {
                return Err(SkillError::Catalog);
            }
            let mut curves = BTreeMap::new();
            for line in lines {
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if line.len() > 256 * 1024 {
                    return Err(SkillError::Catalog);
                }
                let fields: Vec<_> = line.split('\t').collect();
                if fields.len() != 4
                    || fields[2].len() != 64
                    || !fields[2].bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    return Err(SkillError::Catalog);
                }
                let id = u32::from_str_radix(fields[0], 16).map_err(|_| SkillError::Catalog)?;
                let fingerprint =
                    u64::from_str_radix(fields[1], 16).map_err(|_| SkillError::Catalog)?;
                let mut xp = Vec::new();
                for value in fields[3].split(',') {
                    let value = value.parse::<u64>().map_err(|_| SkillError::Catalog)?;
                    if value > i64::MAX as u64
                        || xp.len() >= 10001
                        || xp.last().is_some_and(|previous| *previous > value)
                    {
                        return Err(SkillError::Catalog);
                    }
                    xp.push(value);
                }
                if id >> 24 != 0x6f
                    || xp.len() < 2
                    || xp[0] != 0
                    || curves.len() >= 256
                    || curves.insert(id, Curve { fingerprint, xp }).is_some()
                {
                    return Err(SkillError::Catalog);
                }
            }
            Ok(curves)
        })
        .as_ref()
        .map_err(Clone::clone)
}
pub(super) fn get(id: u32) -> Result<&'static Curve, SkillError> {
    curves()?.get(&id).ok_or(SkillError::AdvancementUnavailable)
}
pub(super) fn validate(portal: &mut Dat) -> std::io::Result<()> {
    for (id, curve) in curves().map_err(std::io::Error::other)? {
        if fingerprint(&portal.read(*id)?) != curve.fingerprint {
            return Err(std::io::Error::other(format!(
                "skill advancement record changed: {id:#010x}"
            )));
        }
    }
    Ok(())
}
