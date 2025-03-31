# phd: Personal Health Daemon

**BIG FAT WARNING. This is ALPHA software. It can brick your devices. Use at your own risk.**

This is an attempt to read measurements from healthcare units and send them to InfluxDB.

## Supported Devices

| Device          | Type                   |
|-----------------|------------------------|
| Omron HEM-7361T | Blood Pressure Monitor |
| Omron HN-300T2  | Weight Scale           |

At the moment, all the measurements are fetched, not just the unread ones.

## System Requirements

- Any recent Linux distro
- Supported Bluetooth adapter
- Enable [experimental](https://github.com/bluez/bluer/issues/110) bluetoothd feature to enable passive scanning
- [InfluxDB v2](https://docs.influxdata.com/influxdb/v2) to store measurements

## Build

- D-Bus and OpenSSL headers/libs, e.g. on Debian/Ubuntu:
  > apt install libdbus-1-dev libssl-dev
- Recent rust, see [rustup](https://rustup.rs)
- Run build:
  > cargo build

## Config file

The config file is in .yaml format, adjust it to your setup, e.g.:

```
devices:
  - id: my_bpm
    driver_config:
      driver: Omron_HEM_7361T
      addr: 34:f7:f2:15:29:ca # Bluetooth address of the unit
      secret: deadbeefdeadbeefdeadbeefdeadbeef # In order to read measurements from the unit, a secret (16 bytes) key is written during pairing, please generate your own random secret
      tz: Europe/Budapest # When sending current date/time to unit, use this timezone
    meas: blood_pressure # InfluxDB measurement name

  - id: my_scale
    driver_config:
      driver: Omron_HN_300T2
      addr: e2:81:4c:12:19:bc # Bluetooth address of the unit
      tz: Europe/Budapest # When sending current date/time to unit, use this timezone
    sleep: 3600 # Optional: after successful data retrieval from the unit, sleep 1 hour (useful if the unit sends BLE advertisement often)
    meas: weight # InfluxDB measurement name

db: # InfluxDB connection settings
  url: http://localhost:8086
  token: abcdefblabla==
  org: org_name
  bucket: bucket_name
```  

## Pair with device

Devices in config.yaml needs to be paired first. Put your device in pairing mode (see instruction manual) and execute:

> cargo run -- -c config.yaml -p my_bpm

## Run daemon in the foreground

The daemon will log into stdout/stderr:

> cargo run -- -c config.yaml
