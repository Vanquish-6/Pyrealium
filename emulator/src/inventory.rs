//! Bounded single-owner inventory transitions. No packet fields grant item authority.
use std::collections::BTreeMap;

pub const ITEM_NAMESPACE: u64 = 0x8130000000000000;
pub const BAG_SLOTS: u16 = 78; // Local policy: 42 + 12 + 12 + 12.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Location {
    Bag(u16),
    Equipped(u16),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: u64,
    pub definition: u32,
    pub visual: u32,
    pub quantity: u16,
    pub stack_limit: u16,
    pub equipment_mask: u16,
    pub location: Location,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inventory {
    items: BTreeMap<u64, Item>,
    high_water: u64,
    // Derived indexes are rebuilt on candidates before Store can publish them.
    // They never enter the journal or provide a second source of ownership.
    bags: [Option<u64>; BAG_SLOTS as usize],
    equipment: [Option<u64>; 11],
}
impl Default for Inventory {
    fn default() -> Self {
        Self {
            items: BTreeMap::new(),
            high_water: 0,
            bags: [None; BAG_SLOTS as usize],
            equipment: [None; 11],
        }
    }
}
#[derive(Clone, Debug)]
pub enum Command {
    Grant(Item),
    Move {
        item: u64,
        source: u16,
        destination: u16,
    },
    /// Dense native bag-list insertion. Destination is the pre-removal boundary.
    Reorder {
        item: u64,
        source: u16,
        destination: u16,
    },
    Equip {
        item: u64,
        slot: u16,
    },
    /// Native dense-list removal into an empty equipment slot. Kept distinct
    /// from historical `Equip` journal semantics, which can leave bag holes.
    Wear {
        item: u64,
        slot: u16,
    },
    Unequip {
        item: u64,
    },
    Split {
        item: u64,
        new_id: u64,
        quantity: u16,
        destination: u16,
    },
    Consume {
        item: u64,
        quantity: u16,
    },
}
#[derive(Debug)]
pub enum InventoryError {
    InvalidItem,
    NotOwned,
    StaleLocation,
    NoSpace,
    IncompatibleEquipment,
    InvalidStack,
}
impl std::fmt::Display for InventoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for InventoryError {}
impl Inventory {
    pub fn items(&self) -> impl Iterator<Item = &Item> {
        self.items.values()
    }
    pub fn high_water(&self) -> u64 {
        self.high_water.max(ITEM_NAMESPACE)
    }
    pub fn free_slot(&self) -> Option<u16> {
        self.bags
            .iter()
            .position(Option::is_none)
            .map(|slot| slot as u16)
    }
    pub fn dense_bags(&self) -> bool {
        let count = self.bags.iter().flatten().count();
        self.bags[..count].iter().all(Option::is_some)
    }
    fn at(&self, location: Location) -> Option<u64> {
        match location {
            Location::Bag(slot) => self.bags.get(usize::from(slot)).copied().flatten(),
            Location::Equipped(slot) if slot.is_power_of_two() && slot <= 0x400 => {
                self.equipment[slot.trailing_zeros() as usize]
            }
            Location::Equipped(_) => None,
        }
    }
    fn reindex(&mut self) -> Result<(), InventoryError> {
        self.bags.fill(None);
        self.equipment.fill(None);
        for item in self.items.values() {
            let occupied = match item.location {
                Location::Bag(slot) => self.bags.get_mut(usize::from(slot)),
                Location::Equipped(slot) if slot.is_power_of_two() && slot <= 0x400 => {
                    self.equipment.get_mut(slot.trailing_zeros() as usize)
                }
                Location::Equipped(_) => None,
            }
            .ok_or(InventoryError::StaleLocation)?;
            if occupied.replace(item.id).is_some() {
                return Err(InventoryError::StaleLocation);
            }
        }
        Ok(())
    }
    fn owned(&self, id: u64) -> Result<&Item, InventoryError> {
        self.items.get(&id).ok_or(InventoryError::NotOwned)
    }
    /// Candidate state is published by Store only after its event has been flushed.
    pub fn apply(&self, command: &Command) -> Result<Self, InventoryError> {
        let mut next = self.clone();
        match command {
            Command::Grant(item) => {
                if item.id <= self.high_water()
                    || item.id >> 48 != ITEM_NAMESPACE >> 48
                    || item.definition >> 24 != 0x47
                    || item.visual == 0
                    || item.quantity == 0
                    || item.stack_limit == 0
                    || item.quantity > item.stack_limit
                    || item.equipment_mask & !0x7ff != 0
                    || !matches!(item.location,Location::Bag(slot) if slot<BAG_SLOTS)
                {
                    return Err(InventoryError::InvalidItem);
                }
                if self.at(item.location).is_some() {
                    return Err(InventoryError::NoSpace);
                }
                next.high_water = item.id;
                next.items.insert(item.id, item.clone());
            }
            Command::Move {
                item,
                source,
                destination,
            } => {
                if *source >= BAG_SLOTS || *destination >= BAG_SLOTS {
                    return Err(InventoryError::StaleLocation);
                }
                let moved = self.owned(*item)?;
                if moved.location != Location::Bag(*source) {
                    return Err(InventoryError::StaleLocation);
                }
                if source == destination {
                    return Ok(next);
                }
                if let Some(other_id) = self.at(Location::Bag(*destination)) {
                    let other = &self.items[&other_id];
                    if (
                        moved.definition,
                        moved.visual,
                        moved.stack_limit,
                        moved.equipment_mask,
                    ) == (
                        other.definition,
                        other.visual,
                        other.stack_limit,
                        other.equipment_mask,
                    ) && moved.stack_limit > 1
                    {
                        let total = u32::from(moved.quantity) + u32::from(other.quantity);
                        if total > u32::from(other.stack_limit) {
                            return Err(InventoryError::InvalidStack);
                        }
                        next.items.get_mut(&other_id).unwrap().quantity = total as u16;
                        next.items.remove(item);
                    } else {
                        next.items.get_mut(item).unwrap().location = Location::Bag(*destination);
                        next.items.get_mut(&other_id).unwrap().location = Location::Bag(*source);
                    }
                } else {
                    next.items.get_mut(item).unwrap().location = Location::Bag(*destination);
                }
            }
            Command::Reorder {
                item,
                source,
                destination,
            } => {
                let moved = self.owned(*item)?;
                if moved.location != Location::Bag(*source) {
                    return Err(InventoryError::StaleLocation);
                }
                let mut order: Vec<u64> = self.bags.iter().flatten().copied().collect();
                // Captures establish insertion in a dense list, not how holes
                // should compact. Refuse sparse bags rather than moving them.
                if self.bags[..order.len()].iter().any(Option::is_none)
                    || usize::from(*source) >= order.len()
                    || usize::from(*destination) > order.len()
                {
                    return Err(InventoryError::StaleLocation);
                }
                let destination = usize::from(*destination) - usize::from(source < destination);
                order.remove(usize::from(*source));
                order.insert(destination, *item);
                for (slot, id) in order.into_iter().enumerate() {
                    next.items.get_mut(&id).unwrap().location = Location::Bag(slot as u16);
                }
            }
            Command::Wear { item, slot } => {
                let order: Vec<u64> = self.bags.iter().flatten().copied().collect();
                if self.bags[..order.len()].iter().any(Option::is_none)
                    || self.at(Location::Equipped(*slot)).is_some()
                    || !matches!(self.owned(*item)?.location, Location::Bag(_))
                {
                    return Err(InventoryError::StaleLocation);
                }
                next = self.apply(&Command::Equip {
                    item: *item,
                    slot: *slot,
                })?;
                for (slot, id) in order.into_iter().filter(|id| id != item).enumerate() {
                    next.items.get_mut(&id).unwrap().location = Location::Bag(slot as u16);
                }
            }
            Command::Equip { item, slot } => {
                let equipped = self.owned(*item)?;
                if !slot.is_power_of_two()
                    || *slot > 0x400
                    || equipped.equipment_mask & slot == 0
                    || equipped.quantity != 1
                {
                    return Err(InventoryError::IncompatibleEquipment);
                }
                if equipped.location == Location::Equipped(*slot) {
                    return Ok(next);
                }
                let Location::Bag(source) = equipped.location else {
                    return Err(InventoryError::StaleLocation);
                };
                if let Some(previous) = self.at(Location::Equipped(*slot)) {
                    next.items.get_mut(&previous).unwrap().location = Location::Bag(source);
                }
                next.items.get_mut(item).unwrap().location = Location::Equipped(*slot);
            }
            Command::Unequip { item } => {
                if !matches!(self.owned(*item)?.location, Location::Equipped(_)) {
                    return Err(InventoryError::StaleLocation);
                }
                let destination = self.free_slot().ok_or(InventoryError::NoSpace)?;
                next.items.get_mut(item).unwrap().location = Location::Bag(destination);
            }
            Command::Split {
                item,
                new_id,
                quantity,
                destination,
            } => {
                let original = self.owned(*item)?;
                if !matches!(original.location, Location::Bag(_))
                    || *quantity == 0
                    || *quantity >= original.quantity
                    || *new_id <= self.high_water()
                    || new_id >> 48 != ITEM_NAMESPACE >> 48
                    || *destination >= BAG_SLOTS
                {
                    return Err(InventoryError::InvalidStack);
                }
                if self.at(Location::Bag(*destination)).is_some() {
                    return Err(InventoryError::NoSpace);
                }
                let mut split = original.clone();
                split.id = *new_id;
                split.quantity = *quantity;
                split.location = Location::Bag(*destination);
                next.items.get_mut(item).unwrap().quantity -= quantity;
                next.items.insert(*new_id, split);
                next.high_water = *new_id;
            }
            Command::Consume { item, quantity } => {
                let original = self.owned(*item)?;
                if *quantity == 0
                    || *quantity > original.quantity
                    || !matches!(original.location, Location::Bag(_))
                {
                    return Err(InventoryError::InvalidStack);
                }
                if *quantity == original.quantity {
                    next.items.remove(item);
                } else {
                    next.items.get_mut(item).unwrap().quantity -= quantity;
                }
            }
        }
        next.reindex()?;
        Ok(next)
    }
}
impl Command {
    /// Local journal format; independent of the native serialized request graphs.
    pub fn encode(&self) -> String {
        match self {
            Self::Grant(item) => format!(
                "g,{},{},{},{},{},{},{}",
                item.id,
                item.definition,
                item.visual,
                item.quantity,
                item.stack_limit,
                item.equipment_mask,
                match item.location {
                    Location::Bag(slot) => slot,
                    Location::Equipped(_) => u16::MAX,
                }
            ),
            Self::Move {
                item,
                source,
                destination,
            } => format!("m,{item},{source},{destination}"),
            Self::Reorder {
                item,
                source,
                destination,
            } => {
                format!("r,{item},{source},{destination}")
            }
            Self::Equip { item, slot } => format!("e,{item},{slot}"),
            Self::Wear { item, slot } => format!("w,{item},{slot}"),
            Self::Unequip { item } => format!("u,{item}"),
            Self::Split {
                item,
                new_id,
                quantity,
                destination,
            } => format!("s,{item},{new_id},{quantity},{destination}"),
            Self::Consume { item, quantity } => format!("c,{item},{quantity}"),
        }
    }
    pub fn decode(text: &str) -> Result<Self, InventoryError> {
        if text.len() > 256 {
            return Err(InventoryError::InvalidItem);
        }
        let mut fields = text.split(',');
        let kind = fields.next().ok_or(InventoryError::InvalidItem)?;
        let values: Vec<u64> = fields
            .map(|v| v.parse().map_err(|_| InventoryError::InvalidItem))
            .collect::<Result<_, _>>()?;
        let small = |v: u64| u16::try_from(v).map_err(|_| InventoryError::InvalidItem);
        let word = |v: u64| u32::try_from(v).map_err(|_| InventoryError::InvalidItem);
        Ok(match (kind, values.as_slice()) {
            ("g", [id, did, visual, quantity, limit, mask, slot]) => Self::Grant(Item {
                id: *id,
                definition: word(*did)?,
                visual: word(*visual)?,
                quantity: small(*quantity)?,
                stack_limit: small(*limit)?,
                equipment_mask: small(*mask)?,
                location: Location::Bag(small(*slot)?),
            }),
            ("m", [item, source, destination]) => Self::Move {
                item: *item,
                source: small(*source)?,
                destination: small(*destination)?,
            },
            ("r", [item, source, destination]) => Self::Reorder {
                item: *item,
                source: small(*source)?,
                destination: small(*destination)?,
            },
            ("e", [item, slot]) => Self::Equip {
                item: *item,
                slot: small(*slot)?,
            },
            ("w", [item, slot]) => Self::Wear {
                item: *item,
                slot: small(*slot)?,
            },
            ("u", [item]) => Self::Unequip { item: *item },
            ("s", [item, new_id, quantity, destination]) => Self::Split {
                item: *item,
                new_id: *new_id,
                quantity: small(*quantity)?,
                destination: small(*destination)?,
            },
            ("c", [item, quantity]) => Self::Consume {
                item: *item,
                quantity: small(*quantity)?,
            },
            _ => return Err(InventoryError::InvalidItem),
        })
    }
    pub fn allocated_id(&self) -> Option<u64> {
        match self {
            Self::Grant(item) => Some(item.id),
            Self::Split { new_id, .. } => Some(*new_id),
            _ => None,
        }
    }
}
