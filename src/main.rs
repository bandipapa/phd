use clap::Parser;
use config::{Config, File, FileFormat};
use serde::Deserialize;
use std::collections::HashSet;
use std::process;
use tokio::signal;

mod btutil;

mod db;
use db::{Db, DbConfig, DbPtr};

mod device;
use device::{Device, DeviceConfig};

mod driver;

mod timeutil;

#[derive(Parser)]
#[command(name = clap::crate_name!(), version = clap::crate_version!(), about = clap::crate_description!(), author = clap::crate_authors!())]
struct Args {
    #[arg(short = 'c', long = "config", value_name = "CONFIG", help = "Configuration file")]
    config_fname: String,

    #[arg(short = 'p', long = "pair", value_name = "DEVICE_ID", help = "Pair with device")]
    pair_device_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MainConfig {
    devices: Vec<DeviceConfig>,
    db: DbConfig,
}

// TODO: Use proper logging class.
#[tokio::main]
async fn main() {
    // Parse command line args.

    let args = Args::parse();

    // Parse configuration file.

    let config_builder = match Config::builder()
        .add_source(File::new(&args.config_fname, FileFormat::Yaml))
        .build() {
        Ok(config_builder) => config_builder,
        Err(e) => {
            eprintln!("Unable to open configuration: {}", e);
            process::exit(1);
        }
    };

    let main_config: MainConfig = match config_builder.try_deserialize() {
        Ok(main_config) => main_config,
        Err(e) => {
            eprintln!("Unable to parse configuration: {}", e);
            process::exit(1);
        }
    };

    // Check for unique device ids.

    let mut device_ids = HashSet::new();

    for device_id in main_config.devices.iter().map(|device| device.get_id()) {
        if !device_ids.insert(device_id) {
            eprintln!("Device id is duplicated: {}", device_id);
            process::exit(1);
        }
    }

    // Main logic starts here.
    
    match args.pair_device_id {
        Some(device_id) => {
            // Do pairing.

            match main_config.devices.into_iter().find(|device_config| device_config.get_id() == device_id) {
                Some(device_config) => {
                    let ok = Device::pair(device_config).await;
                    if !ok {
                        process::exit(1);
                    }
                },
                None => {
                    eprintln!("No such device: {}", device_id);
                    process::exit(1);        
                }
            }
        },
        None => {
            // Do main loop.

            println!("daemon starting");

            // Initialize DB.
        
            let db = DbPtr::new(Db::new(main_config.db));
        
            // Start devices.
        
            for device_config in main_config.devices {
                Device::start(DbPtr::clone(&db), device_config);
            }
        
            // TODO: Do proper signal handling, e.g. HUP->reload, TERM->graceful shutdown.
        
            signal::ctrl_c().await.unwrap();        
        }
    }
}
