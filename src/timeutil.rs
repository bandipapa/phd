use chrono::{Datelike, MappedLocalTime, Timelike, TimeZone, Utc};
use serde::de::{self, Deserializer, Visitor};
use std::fmt;
use tzfile::Tz;

struct TzVisitor;

impl<'de> Visitor<'de> for TzVisitor {
    type Value = Tz;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("valid timezone name")
    }

    fn visit_str<E>(self, tz_name: &str) -> Result<Self::Value, E> where E: de::Error {
        Tz::named(tz_name).map_err(|e| E::custom(format!("unable to open timezone: {}", e)))
    }
}

pub struct Current {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub min: u8,
    pub sec: u8,
}

pub struct TimeUtil;

impl TimeUtil {
    pub fn parse_tz<'de, D>(deserializer: D) -> Result<Tz, D::Error> where D: Deserializer<'de> {
        deserializer.deserialize_str(TzVisitor)
    }

    pub fn get_ts(tz: &Tz, year: u16, month: u8, day: u8, hour: u8, min: u8, sec: u8) -> Option<i64> {
        match tz.with_ymd_and_hms(year.into(), month.into(), day.into(), hour.into(), min.into(), sec.into()) {
            MappedLocalTime::Single(datetime) => Some(datetime.timestamp_nanos_opt().unwrap()),
            MappedLocalTime::Ambiguous(_, _) => None,
            MappedLocalTime::None => None,
        }
    }

    pub fn get_current(tz: &Tz) -> Current {
        let datetime = Utc::now().with_timezone(&tz);
    
        Current {
            year: datetime.year().try_into().unwrap(),
            month: datetime.month().try_into().unwrap(),
            day: datetime.day().try_into().unwrap(),
            hour: datetime.hour().try_into().unwrap(),
            min: datetime.minute().try_into().unwrap(),
            sec: datetime.second().try_into().unwrap(),
        }
    }    
}
