use bluer::{Adapter, AdapterEvent, Address, Device, Session};
use bluer::agent::Agent;
use bluer::gatt::remote::{Characteristic, Service};
use bluer::monitor::{Monitor, MonitorEvent, Pattern, RssiSamplingPeriod, Type};
use futures::StreamExt;
use std::fmt;
use std::result;
use uuid::{uuid, Uuid};

const DEVICE_INFO_SERVICE: &Uuid = &uuid!("0000180a-0000-1000-8000-00805f9b34fb");
const MANUFACTURER_CHAR: &Uuid = &uuid!("00002a29-0000-1000-8000-00805f9b34fb");
const MODEL_CHAR: &Uuid = &uuid!("00002a24-0000-1000-8000-00805f9b34fb");
const FIRMWARE_CHAR: &Uuid = &uuid!("00002a26-0000-1000-8000-00805f9b34fb");

pub struct BTDeviceInfo {
    pub manufacturer: String,
    pub model: String,
    #[allow(dead_code)] // TODO: Get serial number as well and print it out during pairing?
    pub firmware: String,
}

pub enum Error {
    Bluetooth(bluer::Error),
    General(String),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            Error::Bluetooth(e) => format!("Bluetooth error: {}", e),
            Error::General(e) => format!("General error: {}", e), // TODO: Rethink error structs.
        };
        formatter.write_str(&s)
    }
}

impl From<bluer::Error> for Error {
    fn from(e: bluer::Error) -> Self {
        Error::Bluetooth(e)
    }
}

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Error::General(String::from(s))
    }
}

pub type Result<T> = result::Result<T, Error>;

pub struct BTUtil;

impl BTUtil {
    pub async fn get_device(addr: &Address, do_disco: bool) -> Result<(Session, Adapter, Device)> {
        let session = Session::new().await?; // TODO: Have single session only.
        let adapter = session.default_adapter().await?;
        let device = adapter.device(*addr)?;

        if do_disco {
            let mut disco = adapter.discover_devices().await?;

            while let Some(ev) = disco.next().await {
                if let AdapterEvent::DeviceAdded(ev_addr) = ev {
                    if ev_addr == *addr {
                        break;
                    }
                }
            }
        }

        Ok((session, adapter, device))
    }

    pub async fn pair(session: &Session, device: &Device) -> Result<()> {
        let agent = Agent { // Accept all requests.
            ..Default::default()
        };
        let _ = session.register_agent(agent).await?;
        
        Ok(device.pair().await?)
    }

    pub async fn wait_for_adv(adapter: &Adapter, device: &Device, pattern: Pattern) -> Result<()> {
        // Passive listen for advertisements.
        
        let mon_mgr = adapter.monitor().await?;

        let mon = Monitor {
            monitor_type: Type::OrPatterns,
            rssi_low_threshold: None,
            rssi_high_threshold: None,
            rssi_low_timeout: None,
            rssi_high_timeout: None,
            rssi_sampling_period: Some(RssiSamplingPeriod::All),
            patterns: Some(vec![pattern]),
            ..Default::default()
        };
        let mut mon_handle = mon_mgr.register(mon).await?;

        while let Some(ev) = mon_handle.next().await {
            if let MonitorEvent::DeviceFound(device_id) = ev {
                if device_id.device == device.address() {
                    return Ok(());
                }
            }
        }

        Err("Failed to receive advertisements".into())
    }

    pub async fn lookup_service(device: &Device, service_uuid: &Uuid) -> Result<Service> {
        let services: Vec<Service> = device.services().await?;

        for service in services.into_iter() {
            if service.uuid().await? == *service_uuid {
                return Ok(service);
            }
        }

        Err("Service not found".into())
    }

    pub async fn lookup_char(service: &Service, char_uuid: &Uuid) -> Result<Characteristic> {
        let chars = service.characteristics().await?;

        for char in chars.into_iter() {
            if char.uuid().await? == *char_uuid {
                return Ok(char);
            }
        }

        Err("Characteristic not found".into())
    }

    pub async fn get_device_info(device: &Device) -> Result<BTDeviceInfo> {
        let service = Self::lookup_service(device, DEVICE_INFO_SERVICE).await?;
        let manufacturer_char = Self::lookup_char(&service, MANUFACTURER_CHAR).await?;
        let model_char = Self::lookup_char(&service, MODEL_CHAR).await?;
        let firmware_char = Self::lookup_char(&service, FIRMWARE_CHAR).await?;

        Ok(BTDeviceInfo {
            manufacturer: Self::get_string(&manufacturer_char).await?,
            model: Self::get_string(&model_char).await?,
            firmware: Self::get_string(&firmware_char).await?,
        })
    }

    async fn get_string(char: &Characteristic) -> Result<String> {
        let data = char.read().await?;

        match String::from_utf8(data) {
            Ok(s) => Ok(s),
            Err(_) => Err("Unable to decode characteristic value".into()),
        }
    }
}
