use pyrealium::{
    server::{self, Config},
    store::Store,
};
use std::{env, net::SocketAddr, path::PathBuf, process::ExitCode};

fn usage() {
    println!(
        "  account-web --auth-data PATH --data GAME_STORE --web-bind 127.0.0.1:19400 --public-origin https://accounts.example.com --public-host game.example.com --public-port 19370\n    Account registration and HTTPS-proxied launcher login backend.\n  account-create --auth-data PATH --account NAME\n    Offline owner account provisioning with hidden password prompts. Stop the account backend first.\n  serve-training --auth-data PATH --bind 0.0.0.0:19370 --game-data PATH\n    Public native-ticket mode; rejects developer/no-ticket authentication."
    );
    println!(
        "{}\n  license / --license: display the complete AGPL-3.0 license.\n",
        pyrealium::licensing::notice()
    );
    println!(
        "  serve --initialize-ui true --portals true --game-data PATH\n    Explicit development instant portal routes; enabled by serve-training."
    );
    println!(
        "  compact-data --data PATH\n    Offline checked journal compaction; retains the preceding journal backup. Server must be stopped."
    );
    println!(
        "  serve --initialize-ui true --lifestones true --game-data PATH\n    Development lifestone binding, @recall, and bound resurrection. Enabled by serve-training."
    );
    println!(
        "  serve-training --game-data PATH [--account local --data PATH --bind 127.0.0.1:9000]\n    Development preset: initialized HUD, bundled training encounter, and once-per-character hand item."
    );
    println!(
        "  serve --initialize-ui true --training-kit true --game-data PATH\n    Explicit development kit: grants one DAT-backed hand item once per human character. Disabled by default."
    );
    println!(
        "  skill-state --character-id N [--event t,SKILL_HEX|g,SKILL_HEX|u,SKILL_HEX|r,SKILL_HEX] --data PATH\n    Offline skill inspection, training, explicit free grant, untraining, or table-backed advancement."
    );
    println!(
        "  player-state --character-id N [--event COMMAND] --data PATH\n    Offline progression inspection/edit: x,XP a,SPEND_XP g,GOLD s,GOLD d,DAMAGE h,HEAL v,COST r,RECOVERY z(resurrect)."
    );
    println!(
        "  serve --game-data PATH [--content PATH|observed|training|none] [--encounters PATH]\n    Encounters use explicit development rules. training selects the authored starter encounter."
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
    if command == "license" || command == "--license" {
        println!(
            "{}\n\n{}",
            pyrealium::licensing::notice(),
            pyrealium::licensing::LICENSE_TEXT
        );
        return Ok(());
    }
    if command == "--help" || command == "help" {
        usage();
        return Ok(());
    }
    let mut bind: SocketAddr = "127.0.0.1:9000".parse()?;
    let mut account = "local".to_string();
    let mut auth_data = None;
    let mut web_bind: SocketAddr = "127.0.0.1:19400".parse()?;
    let mut public_origin = None;
    let mut public_host = None;
    let mut public_port = None;
    let mut data = PathBuf::from("data");
    let mut seconds = None;
    let mut visual = None;
    let mut name = None;
    let mut game_data = None;
    let mut content = None;
    let mut encounters = None;
    let mut landblocks = None;
    let mut record_id = None;
    let mut character_id = None;
    let mut initialize_ui = None;
    let mut training_kit = None;
    let mut lifestones = None;
    let mut portals = None;
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
            "--auth-data" => auth_data = Some(PathBuf::from(value)),
            "--web-bind" => web_bind = value.parse()?,
            "--public-origin" => public_origin = Some(value),
            "--public-host" => public_host = Some(value),
            "--public-port" => public_port = Some(value.parse::<u16>()?),
            "--data" => data = value.into(),
            "--seconds" => seconds = Some(value.parse::<u64>()?),
            "--visual-did" => {
                visual = Some(u32::from_str_radix(value.trim_start_matches("0x"), 16)?)
            }
            "--name" => name = Some(value),
            "--game-data" => game_data = Some(PathBuf::from(value)),
            "--content" => content = Some(PathBuf::from(value)),
            "--encounters" => encounters = Some(PathBuf::from(value)),
            "--landblocks" => landblocks = Some(PathBuf::from(value)),
            "--id" => record_id = Some(u32::from_str_radix(value.trim_start_matches("0x"), 16)?),
            "--character-id" => character_id = Some(value.parse::<u64>()?),
            "--initialize-ui" => initialize_ui = Some(value.parse::<bool>()?),
            "--training-kit" => training_kit = Some(value.parse::<bool>()?),
            "--lifestones" => lifestones = Some(value.parse::<bool>()?),
            "--portals" => portals = Some(value.parse::<bool>()?),
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
        "compact-data" => {
            let mut store = Store::open(&data)?;
            if store.compact_journal()? {
                println!("Journal compacted after replay verification; previous journal retained.");
            } else {
                println!("Journal is already as small as this compaction profile can make it.");
            }
        }
        "skill-state" => {
            let mut store = Store::open(&data)?;
            let id = character_id.ok_or("--character-id is required")?;
            if let Some(event) = progression_event {
                if let Some(skill) = event.strip_prefix("r,") {
                    store.raise_skill(&account, id, u32::from_str_radix(skill, 16)?)?;
                } else {
                    store.skill_command(
                        &account,
                        id,
                        pyrealium::skills::Command::decode(&event)?,
                    )?;
                }
            }
            let saved = store
                .characters
                .get(&id)
                .filter(|c| c.account == account && !c.deleted)
                .ok_or("active character not owned")?;
            println!(
                "{}: {} available skill credits",
                saved.name,
                saved.skills.available_credits(&saved.progression)
            );
            for (id, entry) in saved.skills.entries() {
                println!(
                    "{id:#x}\trank={}\txp={}\tcredits={}\tlocked={}",
                    entry.rank, entry.xp, entry.credits, entry.locked
                );
            }
        }
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
                    MoveRequest::decode(&bytes)?.checked_command(character, &owned.inventory)?
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
        "account-web" => {
            pyrealium::account_web::run(pyrealium::account_web::Config {
                bind: web_bind,
                auth_data: auth_data.ok_or("--auth-data is required")?,
                game_data: data,
                origin: public_origin.ok_or("--public-origin is required")?,
                game_host: public_host.ok_or("--public-host is required")?,
                game_port: public_port.unwrap_or(bind.port()),
            })?;
        }
        "account-stop" => {
            let root = auth_data.ok_or("--auth-data is required")?;
            if !root.join("accounts.lock").is_file() {
                return Err("No account backend lock found.".into());
            }
            std::fs::write(root.join("account-stop.request"), b"stop\n")?;
            wait_for_stop(&root.join("accounts.lock"), seconds.unwrap_or(20))?;
            println!("Account backend stopped.");
        }
        "account-create" => {
            let mut accounts =
                pyrealium::auth::Accounts::open(&auth_data.ok_or("--auth-data is required")?)?;
            let password = zeroize::Zeroizing::new(rpassword::prompt_password("Password: ")?);
            let confirm =
                zeroize::Zeroizing::new(rpassword::prompt_password("Confirm password: ")?);
            if *password != *confirm {
                return Err("Passwords do not match".into());
            }
            accounts.create(&account.to_ascii_lowercase(), &password)?;
            println!("Account created. No password was written to command-line arguments or logs.");
        }
        "serve" | "serve-training" => {
            // Game data means the normal world: HUD, equipment, travel, and combat.
            // A bare serve without game data remains character selection only.
            let world = game_data.is_some() || command == "serve-training";
            if command == "serve-training" && game_data.is_none() {
                return Err("--game-data is required".into());
            }
            if world {
                content.get_or_insert_with(|| PathBuf::from("training"));
                encounters.get_or_insert_with(|| PathBuf::from("training"));
            }
            let initialize_ui = initialize_ui.unwrap_or(world);
            let training_kit = training_kit.unwrap_or(world);
            let lifestones = lifestones.unwrap_or(world);
            let portals = portals.unwrap_or(world);
            server::run_with_auth(
                Config {
                    bind,
                    account,
                    data,
                    run_seconds: seconds,
                    visual_did: visual,
                    game_data,
                    initialize_ui,
                    training_kit,
                    lifestones,
                    portals,
                    content,
                    encounters,
                },
                auth_data,
            )?;
        }
        "stop" => {
            if !data.join("server.lock").is_file() {
                return Err("no server lock found at the selected data directory".into());
            }
            std::fs::write(data.join("stop.request"), b"stop\n")?;
            if let Some(seconds) = seconds {
                wait_for_stop(&data.join("server.lock"), seconds)?;
            }
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

fn wait_for_stop(lock: &std::path::Path, seconds: u64) -> Result<(), Box<dyn std::error::Error>> {
    if !(1..=60).contains(&seconds) {
        return Err("Stop wait must be 1-60 seconds.".into());
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    while lock.exists() {
        if std::time::Instant::now() >= deadline {
            return Err(
                "Server has not stopped; inspect the process before changing its lock.".into(),
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
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
