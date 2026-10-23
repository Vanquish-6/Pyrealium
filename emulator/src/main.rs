use pyrealium::{
    server::{self, Config},
    store::Store,
};
use std::{env, net::SocketAddr, path::PathBuf, process::ExitCode};

fn usage() {
    println!(
        "Pyrealium - AC2 build 7148 emulator\n\n  pyrealium serve [--bind 127.0.0.1:9000] [--account local] [--data data] [--seconds N] [--visual-did HEX] [--game-data PATH]\n  pyrealium stop [--data data]\n  pyrealium character-create --account local --name Explorer [--data data]\n  pyrealium character-create-human --account local --name Explorer [--data data]\n  pyrealium character-list --account local [--data data]\n  pyrealium character-restore --account local --character-id 1 [--data data]\n  pyrealium legacy-inspect --landblocks PATH --id 0299FFFF\n  pyrealium legacy-validate --landblocks PATH\n\nLocal developer authentication only (native auth type 1). Optional DAT world bootstrap is experimental. Character list/restore commands require the server to be stopped. Terrain commands inspect extracted data; they do not establish gameplay compatibility."
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
    let mut landblocks = None;
    let mut record_id = None;
    let mut character_id = None;
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
            "--landblocks" => landblocks = Some(PathBuf::from(value)),
            "--id" => record_id = Some(u32::from_str_radix(value.trim_start_matches("0x"), 16)?),
            "--character-id" => character_id = Some(value.parse::<u64>()?),
            _ => return Err(format!("unknown option {arg}").into()),
        }
    }
    match command.as_str() {
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
