use pyrealium::{
    server::{self, Config},
    store::Store,
};
use std::{env, net::SocketAddr, path::PathBuf, process::ExitCode};

fn usage() {
    println!(
        "  player-state --character-id N [--event COMMAND] --data PATH\n    Offline progression inspection/edit: x,XP g,GOLD s,GOLD d,DAMAGE h,HEAL v,COST r,RECOVERY z(resurrect)."
    );
    println!(
        "  serve --game-data PATH [--content PATH|observed|none]\n    Game-data mode loads the bundled world pack. observed adds captured stationary actors. Use a file for external content or none to disable it."
    );
    println!(
        "  inventory-reorder --character-id N --request PATH\n    Offline import of one reassembled native inventory sorting message (296 bytes)."
    );
    println!(
        "Offline inventory commands (server must be stopped):\n  inventory-list --character-id N\n  inventory-grant --character-id N --game-data PATH --id DEFINITION_HEX --visual-did VISUAL_HEX [--quantity N --stack-limit N --equipment-mask HEX --slot N]\n  inventory-move --character-id N --item-id IID --source-slot N --slot N\n  inventory-equip --character-id N --item-id IID --equipment-mask HEX\n  inventory-unequip --character-id N --item-id IID\n  inventory-split --character-id N --item-id IID --quantity N [--slot N]\n  inventory-consume --character-id N --item-id IID --quantity N\nAll accept --account local --data PATH. These manage stored state; native item replication remains unfinished.\n"
    );
    println!(
        "Pyrealium - AC2 build 7148 emulator\n\n  pyrealium serve [--bind 127.0.0.1:9000] [--account local] [--data data] [--seconds N] [--visual-did HEX] [--game-data PATH] [--initialize-ui true]\n  pyrealium stop [--data data]\n  pyrealium character-create --account local --name Explorer [--data data]\n  pyrealium character-create-human --account local --name Explorer [--data data]\n  pyrealium character-list --account local [--data data]\n  pyrealium character-restore --account local --character-id 1 [--data data]\n  pyrealium legacy-inspect --landblocks PATH --id 0299FFFF\n  pyrealium legacy-validate --landblocks PATH\n\nLocal developer authentication only (native auth type 1). Optional DAT world bootstrap is experimental. Character list/restore commands require the server to be stopped. Terrain commands inspect extracted data; they do not establish gameplay compatibility."
    );
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        usage();
        return Ok(());
    };
    if command == "--help" || command == "help" {
        usage();
        return Ok(());
    }
    let mut bind: SocketAddr = "127.0.0.1:9000".parse()?;
    let mut account = "local".to_string();
    let mut data = PathBuf::from("data");
    let mut seconds = None;
    let mut visual = None;
    let mut name = None;
    let mut game_data = None;
    let mut content = None;
    let mut landblocks = None;
    let mut record_id = None;
    let mut character_id = None;
    let mut initialize_ui = false;
    let mut item_id = None;
    let mut slot = None;
    let mut source_slot = None;
    let mut inventory_request = None;
    let mut progression_event = None;
    let mut quantity = 1u16;
    let mut stack_limit = 1u16;
    let mut equipment_mask = 0u16;
    while let Some(arg) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {arg}"))?;
        match arg.as_str() {
            "--bind" => bind = value.parse()?,
            "--account" => account = value,
            "--data" => data = value.into(),
            "--seconds" => seconds = Some(value.parse::<u64>()?),
            "--visual-did" => {
                visual = Some(u32::from_str_radix(value.trim_start_matches("0x"), 16)?)
            }
            "--name" => name = Some(value),
            "--game-data" => game_data = Some(PathBuf::from(value)),
            "--content" => content = Some(PathBuf::from(value)),
            "--landblocks" => landblocks = Some(PathBuf::from(value)),
            "--id" => record_id = Some(u32::from_str_radix(value.trim_start_matches("0x"), 16)?),
            "--character-id" => character_id = Some(value.parse::<u64>()?),
            "--initialize-ui" => initialize_ui = value.parse::<bool>()?,
            "--item-id" => {
                item_id = Some(if let Some(hex) = value.strip_prefix("0x") {
                    u64::from_str_radix(hex, 16)?
                } else {
                    value.parse()?
                })
            }
            "--slot" => slot = Some(value.parse::<u16>()?),
            "--source-slot" => source_slot = Some(value.parse::<u16>()?),
            "--request" => inventory_request = Some(PathBuf::from(value)),
            "--event" => progression_event = Some(value),
            "--quantity" => quantity = value.parse()?,
            "--stack-limit" => stack_limit = value.parse()?,
            "--equipment-mask" => {
                equipment_mask = u16::from_str_radix(value.trim_start_matches("0x"), 16)?
            }
            _ => return Err(format!("unknown option {arg}").into()),
        }
    }
    match command.as_str() {
        "player-state" => {
            let mut store = Store::open(&data)?;
            let id = character_id.ok_or("--character-id is required")?;
            if let Some(event) = progression_event {
                store.progression_command(
                    &account,
                    id,
                    pyrealium::progression::Command::decode(&event)?,
                )?;
            }
            let character = store
                .characters
                .get(&id)
                .filter(|c| c.account == account && !c.deleted)
                .ok_or("active character not owned")?;
            println!(
                "{} level={} state={:?}",
                character.name,
                character.progression.level().number,
                character.progression
            );
        }
        "inventory-list" | "inventory-grant" | "inventory-move" | "inventory-equip"
        | "inventory-unequip" | "inventory-split" | "inventory-consume" | "inventory-reorder" => {
            use pyrealium::inventory::{Command, Item, Location};
            let mut store = Store::open(&data)?;
            let character = character_id.ok_or("--character-id is required")?;
            let owned = store
                .characters
                .get(&character)
                .filter(|c| c.account == account && !c.deleted)
                .ok_or("active character not owned")?;
            if command == "inventory-list" {
                for item in owned.inventory.items() {
                    println!(
                        "{:#018x}\t{:#010x}\t{}\t{:?}",
                        item.id, item.definition, item.quantity, item.location
                    );
                }
                return Ok(());
            }
            let operation = match command.as_str() {
                "inventory-reorder" => {
                    use pyrealium::inventory_request::{MESSAGE_BYTES, MoveRequest};
                    use std::io::Read;
                    let path = inventory_request.ok_or("--request is required")?;
                    let mut bytes = Vec::with_capacity(MESSAGE_BYTES + 1);
                    std::fs::File::open(path)?
                        .take((MESSAGE_BYTES + 1) as u64)
                        .read_to_end(&mut bytes)?;
                    MoveRequest::decode(&bytes)?.command(character)?
                }
                "inventory-grant" => {
                    let directory =
                        game_data.ok_or("--game-data is required to validate grant assets")?;
                    let definition =
                        record_id.ok_or("--id must specify the item definition DID")?;
                    let visual = visual.ok_or("--visual-did is required")?;
                    let mut portal = pyrealium::dat::Dat::open(directory.join("portal.dat"))?;
                    portal.read(definition)?;
                    portal.read(visual)?;
                    let id = store.next_item_id()?;
                    let destination = slot
                        .or_else(|| owned.inventory.free_slot())
                        .ok_or("inventory is full")?;
                    println!(
                        "Development grant: {id:#018x}; stack/equipment rules supplied explicitly by the operator."
                    );
                    Command::Grant(Item {
                        id,
                        definition,
                        visual,
                        quantity,
                        stack_limit,
                        equipment_mask,
                        location: Location::Bag(destination),
                    })
                }
                "inventory-move" => Command::Move {
                    item: item_id.ok_or("--item-id is required")?,
                    source: source_slot.ok_or("--source-slot is required")?,
                    destination: slot.ok_or("--slot is required")?,
                },
                "inventory-equip" => Command::Equip {
                    item: item_id.ok_or("--item-id is required")?,
                    slot: equipment_mask,
                },
                "inventory-unequip" => Command::Unequip {
                    item: item_id.ok_or("--item-id is required")?,
                },
                "inventory-split" => Command::Split {
                    item: item_id.ok_or("--item-id is required")?,
                    new_id: store.next_item_id()?,
                    quantity,
                    destination: slot
                        .or_else(|| owned.inventory.free_slot())
                        .ok_or("inventory is full")?,
                },
                "inventory-consume" => Command::Consume {
                    item: item_id.ok_or("--item-id is required")?,
                    quantity,
                },
                _ => unreachable!(),
            };
            store.inventory_command(&account, character, operation)?;
            println!(
                "Inventory change saved. Native inventory initialization requires --initialize-ui true and remains unverified."
            );
        }
        "character-restore" => {
            let mut store = Store::open(&data)?;
            let id = character_id.ok_or("--character-id is required")?;
            store.restore(&account, id)?;
            println!("Restored character {id} for {account}.");
        }
        "character-list" => {
            let store = Store::open(&data)?;
            for c in store.characters.values().filter(|c| c.account == account) {
                println!(
                    "{}\t{}\t{}",
                    c.id,
                    if c.deleted { "deleted" } else { "active" },
                    c.name
                );
            }
        }
        "legacy-validate" => {
            let root = landblocks.ok_or("--landblocks is required")?;
            let result = pyrealium::legacy::validate_terrain(&root)?;
            println!(
                "Validated {} terrain records; {} missing/empty linked info records. Terrain format validation only; gameplay compatibility is unverified.",
                result.terrain_records, result.missing_info_records
            );
            if result.missing_info_records > 0 {
                return Err("legacy terrain has missing/empty linked info records".into());
            }
        }
        "legacy-inspect" => {
            let root = landblocks.ok_or("--landblocks is required")?;
            let id = record_id.ok_or("--id is required")?;
            let bytes = pyrealium::legacy::read_record(&root, id)?;
            let terrain = pyrealium::legacy::Terrain::decode(&bytes, id)?;
            println!(
                "Terrain 0x{:08X}: 17 x 17 samples, info 0x{:08X}, raw height range {:?}, {} bytes. Raw heights require the separate height table for world-space elevation.",
                terrain.id,
                terrain.info_id,
                terrain.height_range(),
                bytes.len()
            );
        }
        "serve" => server::run(Config {
            bind,
            account,
            data,
            run_seconds: seconds,
            visual_did: visual,
            game_data,
            initialize_ui,
            content,
        })?,
        "stop" => {
            if !data.join("server.lock").is_file() {
                return Err("no server lock found at the selected data directory".into());
            }
            std::fs::write(data.join("stop.request"), b"stop\n")?;
            println!("Graceful stop requested.");
        }
        "character-create" | "character-create-human" => {
            let mut store = Store::open(&data)?;
            let name = name.ok_or("--name is required")?;
            let c = if command == "character-create-human" {
                store.create_native(
                    &account,
                    &pyrealium::world::CharacterCreationRequest {
                        name,
                        entity_did: pyrealium::bootstrap::PLAYER_ENTITY,
                        start_area: 0,
                        race: 1,
                        sex: 0x1000,
                        class: 0,
                        implement: 0,
                        physical_values: Vec::new(),
                    },
                )?
            } else {
                store.create(&account, &name)?
            };
            println!(
                "Saved character {} ({}) for {}. World entry requires the matching local DAT bootstrap profile.",
                c.name, c.id, c.account
            );
        }
        _ => return Err(format!("unknown command {command}").into()),
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
