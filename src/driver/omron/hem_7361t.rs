//! # Omron HEM-7361T driver
//! 
//! This driver is based on:
//! - [omblepy](https://github.com/userx14/omblepy)
//! - [ubpm](https://codeberg.org/LazyT/ubpm)

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
const MODEL: &str = "M7 Intelli IT";

const MAIN_SERVICE: &Uuid = &uuid!("ecbe3980-c9a2-11e1-b1bd-0002a5d5c51b");
const UNLOCK_CHAR: &Uuid = &uuid!("b305b680-aee7-11e1-a730-0002a5d5c51b");
const TX_CHARS: &[&Uuid] = &[
    &uuid!("db5b55e0-aee7-11e1-965e-0002a5d5c51b"),
    &uuid!("e0b8a060-aee7-11e1-92f4-0002a5d5c51b"),
    &uuid!("0ae12b00-aee8-11e1-a192-0002a5d5c51b"),
    &uuid!("10e1ba60-aee8-11e1-89e5-0002a5d5c51b")
];
const RX_CHARS: &[&Uuid] = &[
    &uuid!("49123040-aee8-11e1-a74d-0002a5d5c51b"),
    &uuid!("4d0bf320-aee8-11e1-a0d9-0002a5d5c51b"),
    &uuid!("5128ce60-aee8-11e1-b84b-0002a5d5c51b"),
    &uuid!("560f1420-aee8-11e1-8184-0002a5d5c51b")
];

const CMD_CHUNK_SIZE: usize = 0x10;
const SECRET_LEN: usize = 0x10;

const TIMESYNC_ADDR_RD: u16 = 0x003c;
const TIMESYNC_ADDR_WR: u16 = 0x0080;
const TIMESYNC_LEN: usize = 0x10;

const REC_START: &[u16] = &[0x0098, 0x06d8];
const REC_COUNT: usize = 100;
const REC_LEN: usize = 0x10;

const YEAR: u16 = 2000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    addr: Address, // TODO: unique check
    #[serde(deserialize_with = "hex::serde::deserialize")]
    secret: [u8; SECRET_LEN],
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

        // Write secret key.
        
        {
            let mut comm = BTComm::new(&device, MAIN_SERVICE, &[UNLOCK_CHAR], &[UNLOCK_CHAR], CMD_CHUNK_SIZE).await?;

            let mut tx_data = [0_u8; SECRET_LEN + 1];
            tx_data[0] = 0x02;

            let mut rx_data = [0_u8; 2];

            comm.raw(&tx_data, &mut rx_data).await?;
            if rx_data != [0x82, 0x00] {
                return Err("Invalid response".into());
            }

            tx_data[0] = 0x00;
            tx_data[1..].copy_from_slice(&self.config.secret);

            comm.raw(&tx_data, &mut rx_data).await?;
            if rx_data != [0x80, 0x00] {
                return Err("Invalid response".into());
            }
        }

        // Synchronize time.

        {
            let mut comm = BTComm::new(&device, MAIN_SERVICE, TX_CHARS, RX_CHARS, CMD_CHUNK_SIZE).await?;
            comm.start_trans().await?;

            self.sync_time(&mut comm).await?;

            comm.end_trans().await?;
        }

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

        // Unlock device with secret key.

        {
            let mut comm = BTComm::new(&device, MAIN_SERVICE, &[UNLOCK_CHAR], &[UNLOCK_CHAR], CMD_CHUNK_SIZE).await?;

            let mut tx_data = [0_u8; SECRET_LEN + 1];
            tx_data[0] = 0x01;
            tx_data[1..].copy_from_slice(&self.config.secret);

            let mut rx_data = [0_u8; 2];

            comm.raw(&tx_data, &mut rx_data).await?;
            if rx_data != [0x81, 0x00] {
                return Err("Invalid response".into());
            }
        }

        // Exchange data.

        let mut records = DbRecords::new();

        {
            let mut comm = BTComm::new(&device, MAIN_SERVICE, TX_CHARS, RX_CHARS, CMD_CHUNK_SIZE).await?;
            comm.start_trans().await?;

            // Synchronize time.

            self.sync_time(&mut comm).await?;

            // Fetch measurements.
            // TODO: Fetch only unread records

            for (user, start) in REC_START.iter().enumerate() {
                let mut addr = *start;

                for _ in 0..REC_COUNT {
                    let mut data = [0; REC_LEN];
                    let data_len = data.len();

                    if comm.read_eeprom(addr, &mut data, data_len.try_into().unwrap()).await? {
                        let year = YEAR + (data[3] & 0x3f) as u16;
                        let month = (data[5] >> 2) & 0x0f;
                        let day = ((data[4] >> 5) & 0x07) | ((data[5] & 0x03) << 3);
                        let hour = data[4] & 0x1f;
                        let min = ((data[6] >> 6) & 0x03) | ((data[7] & 0x0f) << 2);
                        let sec = data[6] & 0x3f;
                        let bpm = data[2];
                        let dia = data[1];
                        let sys = 25 + data[0];
                        let mov = ((data[5] >> 7) & 0x01) == 0x01;
                        let ihb = ((data[5] >> 6) & 0x01) == 0x01;

                        let ts = TimeUtil::get_ts(&self.config.tz, year, month, day, hour, min, sec).ok_or(btutil::Error::General("Unable to make ts".into()))?;
                        let mut record = DbRecord::new(ts);
                        record.add_tag("user", &format!("{}", user + 1));
                        record.add_field("bpm", DbFieldValue::Integer(bpm.into()));
                        record.add_field("dia", DbFieldValue::Integer(dia.into()));
                        record.add_field("sys", DbFieldValue::Integer(sys.into()));
                        record.add_field("mov", DbFieldValue::Bool(mov));
                        record.add_field("ihb", DbFieldValue::Bool(ihb));
                        
                        records.push(record);
                    }

                    addr += REC_LEN as u16;
                }
            }

            comm.end_trans().await?;
        }

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

        if !comm.read_eeprom(TIMESYNC_ADDR_RD, &mut data, data_len.try_into().unwrap()).await? {
            return Err("Read error".into());
        }

        let current = TimeUtil::get_current(&self.config.tz);
        data[8] = (current.year - YEAR).try_into().unwrap();
        data[9] = current.month;
        data[10] = current.day;
        data[11] = current.hour;
        data[12] = current.min;
        data[13] = current.sec;
        let sum: u16 = data[..14].iter().map(|b| *b as u16).sum();
        data[14] = sum as u8;
        data[15] = 0x00;

        comm.write_eeprom(TIMESYNC_ADDR_WR, &data, data_len.try_into().unwrap()).await
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
