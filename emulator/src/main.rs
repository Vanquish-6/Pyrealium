use pyrealium::{
    server::{self, Config},
    store::Store,
};
use std::{env, net::SocketAddr, path::PathBuf, process::ExitCode};

fn usage() {
    println!(
        "Pyrealium - AC2 local emulator\n\n  pyrealium serve [--bind 127.0.0.1:9000] [--account local] [--data data] [--seconds N] [--visual-did HEX] [--game-data PATH]\n  pyrealium stop [--data data]\n  pyrealium character-create --account local --name Explorer [--data data]\n  pyrealium character-create-human --account local --name Explorer [--data data]\n\nLocal developer authentication only; not an Internet authentication service.\nExperimental world entry requires a supported client/data profile. Movement and gameplay remain unfinished."
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
            _ => return Err(format!("unknown option {arg}").into()),
        }
    }
    match command.as_str() {
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
