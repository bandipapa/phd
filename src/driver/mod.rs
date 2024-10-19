use async_trait::async_trait;
use serde::Deserialize;

use crate::db::DbRecords;

mod omron;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "driver")]
#[allow(non_camel_case_types)]
pub enum DriverConfig { // Keep enum sorted and grouped by manufacturer.
    Omron_HEM_7361T(omron::hem_7361t::Config),
    Omron_HN_300T2(omron::hn_300t2::Config),
}

#[async_trait]
pub trait Driver { // TODO: Have "driver-classes" to simplify coding of additional drivers/reduce boilerplate code?
    async fn pair(&self) -> Result<(), String>;
    async fn get_records(&self) -> Result<DbRecords, String>;
}

pub fn create(id: &str, config: DriverConfig) -> Box<dyn Driver + Send> { // Send is needed because of async.
    // TODO: replace id parameter with logger(?)
    match config {
        DriverConfig::Omron_HEM_7361T(config) => Box::new(omron::hem_7361t::DriverImpl::new(id, config)),
        DriverConfig::Omron_HN_300T2(config) => Box::new(omron::hn_300t2::DriverImpl::new(id, config)),
    }
}
