//! # Omron specific RX/TX routines

use bluer::Device;
use bluer::gatt::remote::Characteristic;
use futures::{Stream, StreamExt};
use std::iter;
use std::pin::Pin;
use uuid::Uuid;

use crate::btutil::{self, BTUtil};

const PKT_HDR_SIZE: usize = 4; // Including len, op and crc.

pub struct BTComm {
    tx_chars: Vec<Characteristic>,
    rx_streams: Vec<BTCommRxStream>,
    cmd_chunk_size: usize,
}

type BTCommRxStream = Pin<Box<dyn Stream<Item = Vec<u8>> + Send>>; // See return value of Characteristic->notify().

pub struct BTCommCmdResp {
    op: u16,
    data: Vec<u8>,
}

impl BTComm {
    // TODO: Implement retry and timeout for bt operations.
    // TODO: connect timeout/pair timeout.

    pub async fn new(device: &Device, service_uuid: &Uuid, tx_char_uuids: &[&Uuid], rx_char_uuids: &[&Uuid], cmd_chunk_size: usize) -> btutil::Result<Self> {
        assert!(!tx_char_uuids.is_empty() && !rx_char_uuids.is_empty());
        let service = BTUtil::lookup_service(device, service_uuid).await?;

        // Obtain characteristic for TX.

        let mut tx_chars = Vec::new();

        for tx_char_uuid in tx_char_uuids {
            let tx_char = BTUtil::lookup_char(&service, tx_char_uuid).await?;
            tx_chars.push(tx_char);
        }

        // Obtain streams for RX.

        let mut rx_streams = Vec::new();

        for rx_char_uuid in rx_char_uuids {
            let rx_char = BTUtil::lookup_char(&service, rx_char_uuid).await?;
            let rx_stream = rx_char.notify().await?;
            let rx_stream: BTCommRxStream = Box::pin(rx_stream);
            rx_streams.push(rx_stream);
        }

        Ok(Self {
            tx_chars,
            rx_streams,
            cmd_chunk_size,
        })
    }

    pub async fn raw(&mut self, tx_data: &[u8], rx_data: &mut [u8]) -> btutil::Result<()> {
        // Write data.

        assert!(self.tx_chars.len() == 1 && self.rx_streams.len() == 1);
        self.tx_chars[0].write(tx_data).await?;

        // Read data.

        match self.rx_streams[0].next().await {
            Some(buf) => {
                let rx_data_len = rx_data.len();

                if buf.len() < rx_data_len {
                    return Err("Received packet is too short".into());
                }

                rx_data.copy_from_slice(&buf[..rx_data_len]);
                Ok(())
            },
            None => Err("Unable to receive packet".into()),
        }
    }

    pub async fn cmd(&mut self, op: u16, data: &[u8]) -> btutil::Result<BTCommCmdResp> {
        // Construct packet.

        let pkt_len = data.len() + PKT_HDR_SIZE;
        assert!(pkt_len <= self.tx_chars.len() * self.cmd_chunk_size);

        let mut pkt = Vec::new();
        pkt.push(pkt_len.try_into().unwrap()); // Make sure we fit in u8.
        pkt.push((op >> 8) as u8);
        pkt.push((op & 0xff) as u8);
        pkt.extend_from_slice(data);
        pkt.push(Self::crc(&pkt));
        assert!(pkt_len == pkt.len());

        // Write command.

        for (tx_char, buf) in iter::zip(&self.tx_chars, pkt.chunks(self.cmd_chunk_size)) {
            tx_char.write(buf).await?;
        }

        // Receive response.

        let mut pkt = Vec::new();
        let mut pkt_len: usize = 0;

        for (i, rx_stream) in self.rx_streams.iter_mut().enumerate() {
            let buf = match rx_stream.next().await {
                Some(buf) => buf,
                None => return Err("Unable to receive packet".into()),
            };

            if i == 0 { // First chunk.
                if buf.is_empty() {
                    return Err("Received packet is too short".into());
                }

                pkt_len = buf[0].into();
                if pkt_len < PKT_HDR_SIZE {
                    return Err("Received packet is too short".into());
                }
            }

            pkt.extend_from_slice(&buf);
            if pkt.len() >= pkt_len {
                break;
            }
        }

        if pkt.len() < pkt_len {
            return Err("Received packet is too short".into());
        }

        pkt.truncate(pkt_len);

        // Process response.

        if Self::crc(&pkt) != 0 {
            return Err("CRC error in received packet".into());
        }

        let op = (pkt[1] as u16) << 8 | (pkt[2] as u16);
        let data_len = pkt_len - PKT_HDR_SIZE;
        let data = Vec::from(&pkt[3..3 + data_len]);

        Ok(BTCommCmdResp {
            op,
            data,
        })
    }

    pub async fn start_trans(&mut self) -> btutil::Result<()> {
        let resp = self.cmd(0x0000, &[0x00, 0x00, 0x10, 0x00]).await?;
        if resp.op != 0x8000 {
            return Err("Invalid response".into());
        }

        Ok(())
    }

    pub async fn end_trans(&mut self) -> btutil::Result<()> {
        let resp = self.cmd(0x0f00, &[0x00, 0x00, 0x00, 0x00]).await?;
        if resp.op != 0x8f00 {
            return Err("Invalid response".into());
        }

        Ok(())
    }

    pub async fn read_eeprom(&mut self, start: u16, data: &mut [u8], block_size: u8) -> btutil::Result<bool> {
        assert!(block_size > 0);

        let mut cmd_data = Vec::new(); // Allocate command buffer.
        let mut addr = start;

        for buf in data.chunks_mut(block_size.into()) {
            let todo = buf.len();
            assert!(todo > 0);

            cmd_data.clear();
            cmd_data.push((addr >> 8) as u8);
            cmd_data.push((addr & 0xff) as u8);
            cmd_data.push(todo.try_into().unwrap()); // Make sure we fit in u8.
            cmd_data.push(0x00);

            let resp = self.cmd(0x0100, &cmd_data).await?;
            let resp_data = resp.data;
            let resp_data_len = resp_data.len();

            if resp.op != 0x8100 || resp_data_len < 3 {
                return Err("Invalid response".into());
            }

            let resp_addr = (resp_data[0] as u16) << 8 | (resp_data[1] as u16);
            let resp_todo = resp_data[2] as usize;
            if resp_addr != addr || resp_todo != todo {
                return Err("Invalid response".into());
            }

            let expected = 3 + todo;
            if resp_data_len < expected { // TODO: do we need to consider padding?
                return Ok(false);
            }
            buf.copy_from_slice(&resp_data[3..expected]);

            addr += todo as u16;
        }

        Ok(true)
    }

    pub async fn write_eeprom(&mut self, start: u16, data: &[u8], block_size: u8) -> btutil::Result<()> {
        assert!(block_size > 0);

        let mut cmd_data = Vec::new(); // Allocate command buffer.
        let mut addr = start;

        for buf in data.chunks(block_size.into()) {
            let todo = buf.len();
            assert!(todo > 0);

            cmd_data.clear();
            cmd_data.push((addr >> 8) as u8);
            cmd_data.push((addr & 0xff) as u8);
            cmd_data.push(todo.try_into().unwrap()); // Make sure we fit in u8.
            cmd_data.extend_from_slice(buf);
            cmd_data.push(0x00);

            let resp = self.cmd(0x01c0, &cmd_data).await?;
            let resp_data = resp.data;

            if resp.op != 0x81c0 || resp_data.len() < 2 {
                return Err("Invalid response".into());
            }

            let resp_addr = (resp_data[0] as u16) << 8 | (resp_data[1] as u16);
            if resp_addr != addr { // TODO: do we need to check todo (like in read_eeprom)?
                return Err("Invalid response".into());
            }

            addr += todo as u16;
        }

        Ok(())
    }

    fn crc(pkt: &[u8]) -> u8 {
        pkt.iter().fold(0, |acc, b| acc ^ b)
    }
}
