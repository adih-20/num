/*
 * num <https://github.com/adih-20/num>
 * Copyright (C) 2023 Aditya Hadavale
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 */

use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;
use surge_ping::{
    Client, Config, IcmpPacket, PingIdentifier, PingSequence, Pinger, SurgeError, ICMP,
};
use time::format_description::OwnedFormatItem;
use time::{format_description, OffsetDateTime};
use tokio::fs::{File, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::net;

pub struct Engine {
    ip_addr: IpAddr,
    ttl: u32,
    data: Vec<u8>,
    timeout: Duration,
    ping_handler: Pinger,
    start_time: OffsetDateTime,
    last_successful_latency: Option<Duration>,
    last_successful_time: Option<OffsetDateTime>,
    last_failed_time: Option<OffsetDateTime>,
    output_path: PathBuf,
    file_date_fmt: OwnedFormatItem,
    result_file_handle: Option<File>,
}

impl Engine {
    /// Create a new Engine struct and initialize config and result files.
    pub async fn new(
        addr: String,
        ttl_i: u32,
        timeout: u64,
        num_bytes: u8,
        delay: u64,
        path: PathBuf,
    ) -> Self {
        unsafe {
            time::util::local_offset::set_soundness(time::util::local_offset::Soundness::Unsound)
        }
        let ip_addr = Engine::process_ip(addr).await;
        let config = match ip_addr {
            IpAddr::V4(_) => Config::builder().kind(ICMP::V4).ttl(ttl_i).build(),
            IpAddr::V6(_) => Config::builder().kind(ICMP::V6).ttl(ttl_i).build(),
        };
        let client = Client::new(&config).unwrap();
        let mut pinger = client.pinger(ip_addr, PingIdentifier(1)).await;
        pinger.timeout(Duration::from_millis(timeout));
        let mut result_engine = Engine {
            ip_addr,
            data: vec![0; num_bytes.into()],
            timeout: Duration::from_millis(timeout),
            ping_handler: pinger,
            ttl: ttl_i,
            start_time: OffsetDateTime::now_local().expect("TZ data not found for this system"),
            output_path: path,
            last_successful_latency: None,
            last_failed_time: None,
            last_successful_time: None,
            file_date_fmt: format_description::parse_owned::<1>(
                "[month]-[day]-[year]@[hour]-[minute]-[second]",
            )
            .unwrap(),
            result_file_handle: None,
        };
        unsafe {
            time::util::local_offset::set_soundness(time::util::local_offset::Soundness::Sound)
        }
        result_engine.create_config(delay).await;
        result_engine.result_file_handle = Some(result_engine.init_csv().await);
        std::mem::forget(client); // Client's socket needs to survive to ping, so it cannot be dropped
        result_engine
    }

    /// Transmit a ping and log relevant information. Returns sent time and ping information.
    pub async fn ping(&mut self) -> (OffsetDateTime, Result<(IcmpPacket, Duration), SurgeError>) {
        unsafe {
            time::util::local_offset::set_soundness(time::util::local_offset::Soundness::Unsound)
        }
        let curr_time = OffsetDateTime::now_local()
            .expect("TZ data not found for this system or running in multithreaded context");
        unsafe {
            time::util::local_offset::set_soundness(time::util::local_offset::Soundness::Sound)
        }
        let output = self.ping_handler.ping(PingSequence(0), &self.data).await;
        self.write_csv(curr_time, &output).await;
        if let Ok((_, rtt)) = &output {
            self.last_successful_latency = Some(*rtt);
            self.last_successful_time = Some(curr_time);
        } else {
            self.last_failed_time = Some(curr_time);
        }
        (curr_time, output)
    }

    /// Convert a String representation of an IP address or hostname (with/without port number)
    /// to an IpAddr. Panics if invalid address/port number is passed in.
    async fn process_ip(addr: String) -> IpAddr {
        let possible_addr = addr.parse::<IpAddr>();
        if possible_addr.is_err() {
            return if addr.contains(':') {
                net::lookup_host(addr)
                    .await
                    .expect("Address/Port unreachable")
                    .next()
                    .unwrap()
                    .ip()
            } else {
                net::lookup_host([addr, ":80".to_string()].concat())
                    .await
                    .expect("Address/Port unreachable")
                    .next()
                    .unwrap()
                    .ip()
            };
        }
        possible_addr.unwrap()
    }

    /// Creates a JSON file reflecting current application configuration in a user-configurable directory.
    async fn create_config(&self, delay: u64) {
        let js_string = format!("{{\"address\": \"{}\",\"num_bytes\": {},\"timeout\": \"{}ms\",\"ttl\": {},\"delay\": \"{}s\"}}",
            self.ip_addr,
            self.data.len(),
            self.timeout.as_millis(),
            self.ttl,
            delay
        );
        let mut config_file = File::create(self.output_path.join(format!(
            "config_{}.json",
            self.start_time.format(&self.file_date_fmt).unwrap()
        )))
        .await
        .expect("Error creating config file");
        config_file
            .write_all(js_string.as_ref())
            .await
            .expect("Error writing to config file");
        config_file.flush().await.unwrap();
    }

    /// Creates a CSV file for the app logs with a header.
    async fn init_csv(&self) -> File {
        let csv_path = self.output_path.join(format!(
            "result_{}.csv",
            self.start_time.format(&self.file_date_fmt).unwrap()
        ));
        let mut new_csv = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&csv_path)
            .await
            .expect("Error creating CSV");
        new_csv
            .write_all("Timestamp,Latency(ms)\n".as_ref())
            .await
            .expect("Error writing header to CSV");
        new_csv.flush().await.unwrap();
        OpenOptions::new()
            .append(true)
            .open(&csv_path)
            .await
            .unwrap()
    }

    /// Appends log data to a pre-created CSV.
    async fn write_csv(
        &mut self,
        timestamp: OffsetDateTime,
        result: &Result<(IcmpPacket, Duration), SurgeError>,
    ) {
        let rtt: String = match result {
            Ok((_, rtt)) => rtt.as_millis().to_string(),
            Err(_) => "failed".to_string(),
        };
        self.result_file_handle
            .as_mut()
            .unwrap()
            .write_all(format!("{},{}\n", timestamp, rtt).as_ref())
            .await
            .expect("Failed to write to CSV");
        self.result_file_handle
            .as_mut()
            .unwrap()
            .flush()
            .await
            .unwrap();
    }

    pub fn get_last_successful_latency(&self) -> Duration {
        self.last_successful_latency.unwrap()
    }

    pub fn get_possible_last_successful_time(&self) -> Option<OffsetDateTime> {
        self.last_successful_time
    }

    pub fn get_possible_last_failed_time(&self) -> Option<OffsetDateTime> {
        self.last_failed_time
    }

    /// Return the internal IpAddr used for pinging.
    pub fn get_processed_ip(&self) -> IpAddr {
        self.ip_addr
    }
}
