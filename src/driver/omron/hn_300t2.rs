//! # Omron HN-300T2 driver

use async_trait::async_trait;
use bluer::{Address, Device};
use bluer::monitor::{data_type, Pattern};
use serde::Deserialize;
use tzfile::Tz;
use uuid::{uuid, Uuid};

use crate::btutil::{self, BTUtil};
use crate::db::{DbFieldValue, DbRecord, DbRecords};
use crate::driver::Driver;
use crate::timeutil::TimeUtil;
use super::btcomm::BTComm;

const PATTERN_CONTENT: &[u8] = &[0x0e, 0x02];

const MANUFACTURER: &str = "OMRONHEALTHCARE";
const MODEL: &str = "HN300T2IntelliIT";

const MAIN_SERVICE: &Uuid = &uuid!("0000fe4a-0000-1000-8000-00805f9b34fb");
const TX_CHAR: &Uuid = &uuid!("db5b55e0-aee7-11e1-965e-0002a5d5c51b");
const RX_CHAR: &Uuid = &uuid!("49123040-aee8-11e1-a74d-0002a5d5c51b");

const CMD_CHUNK_SIZE: usize = 0xff; // Use large size, so commands are not chunked. // TODO: Use Option<usize>?

const TIMESYNC_ADDR: u16 = 0x0248;
const TIMESYNC_LEN: usize = 0x08;

const REC_START: u16 = 0x02c0;
const REC_COUNT: usize = 30;
const REC_LEN: usize = 0x10;

const YEAR: u16 = 2000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    addr: Address, // TODO: unique check
    #[serde(deserialize_with = "crate::timeutil::TimeUtil::parse_tz")]
    tz: Tz,
}

pub struct DriverImpl {
    id: String,
    config: Config,
}

impl DriverImpl {
    pub fn new(id: &str, config: Config) -> Self {
        Self {
            id: String::from(id),
            config,
        }
    }

    async fn pair(&self) -> btutil::Result<()> {
        // Pair device.

        let (session, _, device) = BTUtil::get_device(&self.config.addr, true).await?;

        if device.is_paired().await? {
            return Err("Device is already paired".into());
        }

        device.connect().await?;
        self.check_device(&device).await?;

        BTUtil::pair(&session, &device).await?;

        // Synchronize time.

        let mut comm = BTComm::new(&device, MAIN_SERVICE, &[TX_CHAR], &[RX_CHAR], CMD_CHUNK_SIZE).await?;
        comm.start_trans().await?;

        self.sync_time(&mut comm).await?;

        comm.end_trans().await?;

        Ok(())
    }

    async fn get_records(&self) -> btutil::Result<DbRecords> {
        // Connect to device.

        let (_, adapter, device) = BTUtil::get_device(&self.config.addr, false).await?;

        if !device.is_paired().await? {
            return Err("Device is not yet paired".into());
        }

        let pattern = Pattern {
            data_type: data_type::MANUFACTURER_SPECIFIC_DATA,
            start_position: 0,
            content: PATTERN_CONTENT.to_vec(),
        };
        BTUtil::wait_for_adv(&adapter, &device, pattern).await?;

        println!("{}: received advertisement, trying to connect", self.id);

        device.connect().await?;
        self.check_device(&device).await?;

        // Exchange data.

        let mut records = DbRecords::new();

        let mut comm = BTComm::new(&device, MAIN_SERVICE, &[TX_CHAR], &[RX_CHAR], CMD_CHUNK_SIZE).await?;
        comm.start_trans().await?;

        // Synchronize time.

        self.sync_time(&mut comm).await?;

        // Fetch measurements.
        // TODO: Fetch only unread records
        //d: [?, 0, 0, 0, ?, 0, 0, 0, 0, ?, 0, ?]
        //    |           |              \---total number of measurements so far
        //    |           \---- & 0x1f: number of available measurements
        //    \-- & 0x1f: next available measurement slot
        //let d = comm.read_eeprom(0x01a0, 0xc).await?.ok_or(btutil::Error::Other(format!("Read error")))?; // 0x0230 write

        let mut addr = REC_START;

        for _ in 0..REC_COUNT {
            let mut data = [0; REC_LEN];
            let data_len = data.len();

            if comm.read_eeprom(addr, &mut data, data_len.try_into().unwrap()).await? {
                let raw_weight = (data[0] as u16) << 8 | (data[1] as u16);
                if raw_weight != 0xffff {
                    let weight = (raw_weight as f64) / 20.0; // Unit reports weight in 50g.
                    let year = YEAR + (data[2] as u16);
                    let month = data[3];
                    let day = data[4];
                    let hour = data[5];
                    let min = data[6];
                    let sec = data[7];

                    let ts = TimeUtil::get_ts(&self.config.tz, year, month, day, hour, min, sec).ok_or(btutil::Error::General("Unable to make ts".into()))?;
                    let mut record = DbRecord::new(ts);
                    record.add_field("weight", DbFieldValue::Float(weight));
                    
                    records.push(record);
                }
            }

            addr += REC_LEN as u16;
        }

        comm.end_trans().await?;

        Ok(records)
    }

    async fn check_device(&self, device: &Device) -> btutil::Result<()> {
        let device_info = BTUtil::get_device_info(device).await?;
        if !(device_info.manufacturer == MANUFACTURER && device_info.model == MODEL) {
            return Err("Unknown device".into());
        }

        Ok(())
    }

    async fn sync_time(&self, comm: &mut BTComm) -> btutil::Result<()> {
        let mut data = [0; TIMESYNC_LEN];
        let data_len = data.len();

        let current = TimeUtil::get_current(&self.config.tz);
        data[0] = (current.year - YEAR).try_into().unwrap();
        data[1] = current.month;
        data[2] = current.day;
        data[3] = current.hour;
        data[4] = current.min;
        data[5] = current.sec;
        let sum: u16 = data.iter().map(|b| *b as u16).sum();
        data[6] = sum as u8;
        data[7] = 0xff;
        
        comm.write_eeprom(TIMESYNC_ADDR, &data, data_len.try_into().unwrap()).await
    }
}

#[async_trait]
impl Driver for DriverImpl {
    async fn pair(&self) -> Result<(), String> {
        self.pair().await.map_err(|e| format!("{}", e))
    }

    async fn get_records(&self) -> Result<DbRecords, String> {
        self.get_records().await.map_err(|e| format!("{}", e))
    }
}
