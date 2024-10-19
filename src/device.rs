use serde::Deserialize;
use tokio::time::{self, Duration};

use crate::db::DbPtr;
use crate::driver::{self, DriverConfig};

const WAIT: u64 = 3; // [s]

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceConfig {
    id: String,
    driver_config: DriverConfig,
    sleep: Option<u32>,
    meas: String,
}

impl DeviceConfig {
    pub fn get_id(&self) -> &str{
        &self.id
    }
}

pub struct Device;

impl Device {
    pub async fn pair(config: DeviceConfig) -> bool {
        let id = config.id;

        println!("{}: pairing", id);

        let driver = driver::create(&id, config.driver_config);

        match driver.pair().await {
            Ok(_) => {
                println!("{}: ok", id);
                true
            },
            Err(e) => {
                eprintln!("{}: {}", id, e);
                false
            }
        }
    }

    pub fn start(db: DbPtr, config: DeviceConfig) {
        tokio::spawn(Self::run(db, config));
    }

    async fn run(db: DbPtr, config: DeviceConfig) {
        let id = config.id;

        println!("{}: starting", id);

        let driver = driver::create(&id, config.driver_config);

        loop {
            let mut records = match driver.get_records().await {
                Ok(records) => records,
                Err(e) => {
                    eprintln!("{}: {}", id, e);
                    Self::wait().await;
                    continue;
                }
            };

            if !records.is_empty() {
                println!("{}: received {} records, sending to DB", id, records.len());

                for record in &mut records {
                    record.add_tag("device_id", &id);
                }

                loop {
                    // TODO: Put records into a queue and have a background task to submit it to influxdb.
                    // TODO: Once commited, update unread status on unit.
                    
                    match db.send(&config.meas, &records).await {
                        Ok(_) => break,
                        Err(e) => {
                            eprintln!("{}: {}", id, e);
                            Self::wait().await;
                        }
                    }
                }

                println!("{}: ok", id);
            }

            if let Some(sleep) = config.sleep {
                time::sleep(Duration::from_secs(sleep.into())).await;
            }
        }
    }

    async fn wait() {
        time::sleep(Duration::from_secs(WAIT)).await;
    }
}
