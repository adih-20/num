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

use ping_rs::{PingApiOutput, PingOptions};
use std::net::{IpAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use time::OffsetDateTime;
use tokio::fs::{File, OpenOptions};
use tokio::io::AsyncWriteExt;

pub struct Engine {
    ip_addr: IpAddr,
    data: Vec<u8>,
    timeout: Duration,
    options: PingOptions,
    start_time: OffsetDateTime,
    last_successful_latency: Option<u32>,
    last_successful_time: Option<OffsetDateTime>,
    last_failed_time: Option<OffsetDateTime>,
    output_path: PathBuf,
}

impl Engine {
    /// Create a new Engine struct and initialize config and result files.
    pub async fn new(
        addr: String,
        ttl_i: u8,
        timeout: u64,
        num_bytes: u8,
        delay: u64,
        path: PathBuf,
    ) -> Self {
        unsafe {
            time::util::local_offset::set_soundness(time::util::local_offset::Soundness::Unsound)
        }
        let result_engine = Engine {
            ip_addr: Engine::process_ip(addr),
            data: vec![0; num_bytes.into()],
            timeout: Duration::from_millis(timeout),
            options: PingOptions {
                ttl: ttl_i,
                dont_fragment: false,
            },
            start_time: OffsetDateTime::now_local().expect("TZ data not found for this system"),
            output_path: path,
            last_successful_latency: None,
            last_failed_time: None,
            last_successful_time: None,
        };
        unsafe {
            time::util::local_offset::set_soundness(time::util::local_offset::Soundness::Sound)
        }
        result_engine.create_config(delay).await;
        result_engine.init_csv().await;
        result_engine
    }

    /// Transmit a ping and log relevant information. Returns sent time and ping information.
    pub async fn ping(&mut self) -> (OffsetDateTime, PingApiOutput) {
        unsafe {
            time::util::local_offset::set_soundness(time::util::local_offset::Soundness::Unsound)
        }
        let curr_time = OffsetDateTime::now_local()
            .expect("TZ data not found for this system or running in multithreaded context");
        unsafe {
            time::util::local_offset::set_soundness(time::util::local_offset::Soundness::Sound)
        }

        let output = ping_rs::send_ping_async(
            &self.ip_addr,
            self.timeout,
            Arc::new(&self.data),
            Some(&self.options),
        )
        .await;

        self.write_csv(curr_time, &output).await;
        if let Ok(output) = &output {
            self.last_successful_latency = Some(output.rtt);
            self.last_successful_time = Some(curr_time);
        } else {
            self.last_failed_time = Some(curr_time);
        }
        (curr_time, output)
    }

    /// Convert a String representation of an IP address or hostname (with/without port number)
    /// to an IpAddr. Panics if invalid address/port number is passed in.
    fn process_ip(addr: String) -> IpAddr {
        addr.parse::<IpAddr>().unwrap_or_else(|_| {
            if addr.contains(':') {
                addr.to_socket_addrs()
                    .expect("Address/Port unreachable")
                    .next()
                    .unwrap()
                    .ip()
            } else {
                [addr, ":80".to_string()]
                    .concat()
                    .to_socket_addrs()
                    .expect("Address/Port unreachable")
                    .next()
                    .unwrap()
                    .ip()
            }
        })
    }

    /// Creates a JSON file reflecting current application configuration in a user-configurable directory.
    async fn create_config(&self, delay: u64) {
        let js_string = format!("{{\"address\": \"{}\",\"num_bytes\": {},\"timeout\": \"{}ms\",\"ttl\": {},\"delay\": \"{}s\"}}",
            self.ip_addr,
            self.data.len(),
            self.timeout.as_millis(),
            self.options.ttl,
            delay
        );
        let mut config_file = File::create(
            self.output_path
                .join(format!("config<{}>.json", self.start_time)),
        )
        .await
        .expect("Error creating config file");
        config_file
            .write_all(js_string.as_ref())
            .await
            .expect("Error writing to config file");
        config_file.flush().await.unwrap();
    }

    /// Creates a CSV file for the app logs with a header.
    async fn init_csv(&self) {
        let mut new_csv = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(
                self.output_path
                    .join(format!("result<{}>.csv", self.start_time)),
            )
            .await
            .expect("Error creating CSV");
        new_csv
            .write_all("Timestamp,Latency(ms)\n".as_ref())
            .await
            .expect("Error writing header to CSV");
        new_csv.flush().await.unwrap();
    }

    /// Appends log data to a pre-created CSV.
    async fn write_csv(&self, timestamp: OffsetDateTime, result: &PingApiOutput) {
        let mut result_csv = OpenOptions::new()
            .append(true)
            .open(
                self.output_path
                    .join(format!("result<{}>.csv", self.start_time)),
            )
            .await
            .expect("Error creating CSV");
        let rtt: String = match result {
            Ok(v) => v.rtt.to_string(),
            Err(_) => "failed".to_string(),
        };
        result_csv
            .write_all(format!("{},{}\n", timestamp, rtt).as_ref())
            .await
            .expect("Failed to write to CSV");
        result_csv.flush().await.unwrap();
    }

    pub fn get_last_successful_latency(&self) -> u32 {
        self.last_successful_latency.unwrap()
    }

    pub fn get_possible_last_successful_time(&self) -> Option<OffsetDateTime> {
        self.last_successful_time
    }

    pub fn get_possible_last_failed_time(&self) -> Option<OffsetDateTime> {
        self.last_failed_time
    }
}
