use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DbConfig {
    url: String,
    token: String,
    org: String,
    bucket: String,
}

pub struct DbRecord {
    ts: i64, // Timestamp [ns]
    tags: HashMap<String, String>,
    fields: HashMap<String, DbFieldValue>,
}

pub type DbRecords = Vec<DbRecord>;

pub enum DbFieldValue {
    Float(f64),
    Integer(i64),
    Bool(bool),
}

impl DbRecord {
    pub fn new(ts: i64) -> Self {
        Self {
            ts,
            tags: HashMap::new(),
            fields: HashMap::new()
        }
    }

    pub fn add_tag(&mut self, key: &str, value: &str) {
        self.tags.insert(String::from(key), String::from(value));
    }

    pub fn add_field(&mut self, key: &str, value: DbFieldValue) {
        self.fields.insert(String::from(key), value);
    }
}

pub struct Db {
    config: DbConfig,
}

pub type DbPtr = Arc<Db>;

impl Db {
    pub fn new(config: DbConfig) -> Self {
        Self {
            config,
        }
    }

    pub async fn send(&self, meas: &str, records: &[DbRecord]) -> Result<(), String> {
        assert!(!records.is_empty());

        // Construct body.

        let body = records.iter().map(|record| { // TODO: escape tags and fields
            assert!(!record.fields.is_empty());

            format!("{}{} {} {}\n",
                meas,
                record.tags.iter().map(|(key, value)| format!(",{}={}", key, value)).collect::<Vec<String>>().join(""),
                record.fields.iter().map(|(key, value)| format!("{}={}",
                    key,
                    match value {
                        DbFieldValue::Float(value) => format!("{}", value),
                        DbFieldValue::Integer(value) => format!("{}", value),
                        DbFieldValue::Bool(value) => String::from(if *value { "true" } else { "false" }),
                    }
                )).collect::<Vec<String>>().join(","),
                record.ts
            )
        }).collect::<Vec<String>>().join("");

        // Send request.

        let client = Client::new();

        match client.post(format!("{}/api/v2/write", self.config.url))
            .query(&[
                ("org", self.config.org.as_ref()),
                ("bucket", self.config.bucket.as_ref()),
                ("precision", "ns"),
            ])
            .header("Authorization", format!("Token {}", self.config.token))
            .header("Content-Type", "text/plain; charset=utf-8")
            .header("Accept", "application/json")
            .body(body)
            .send()
            .await {
            Ok(_) => Ok(()),
            Err(e) => Err(format!("DB error: {}", e)),
        }
    }
}
