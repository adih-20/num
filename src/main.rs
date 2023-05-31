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

use crate::engine::Engine;
use clap::{arg, value_parser, Command};
use crossterm::style::{Attribute, StyledContent, Stylize};
use crossterm::{cursor, terminal, ExecutableCommand};
use ping_rs::PingApiOutput;
use std::io::{stdout, Stdout, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use time::format_description::FormatItem;
use time::{format_description, OffsetDateTime};
use tokio::{signal, task};
mod engine;

// Format string for user-presented timestamp
const DT_FMT: &str = "[month]/[day]/[year] [hour]:[minute]:[second]";

#[tokio::main(flavor = "current_thread")]
async fn main() {
    // Set up argument parser
    let matches = Command::new("num (Network Uptime Monitor)")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Monitors the uptime of a network connection and records data to a CSV.")
        .arg(arg!(<ADDRESS> "Host to ping (required)").required(true))
        .arg(
            arg!(-o --output <PATH> "Output directory path (required)")
                .required(true)
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            arg!(-t --timeout <TIMEOUT> "Time to wait for host response (ms) (default=1000)")
                .required(false)
                .value_parser(value_parser!(u64)),
        )
        .arg(
            arg!(-d --delay <DELAY> "Time to wait between pings (s) (default=120, min=5)")
                .required(false)
                .value_parser(value_parser!(u64).range(5..)),
        )
        .arg(
            arg!(-n --"num-bytes" <BYTES> "Number of bytes to send (default=4, max=24)") // due to ping_rs restrictions
                .required(false)
                .value_parser(value_parser!(u8).range(1..25)),
        )
        .arg(
            arg!(--ttl <TTL> "Set the ping Time to Live (default=128, max=255)")
                .required(false)
                .value_parser(value_parser!(u8).range(1..)),
        )
        .get_matches();

    // Extract values from parser
    let addr = matches.get_one::<String>("ADDRESS").unwrap().to_string();
    let output_path = matches.get_one::<PathBuf>("output").unwrap().to_path_buf();
    let timeout = matches
        .get_one::<u64>("timeout")
        .unwrap_or(&1000)
        .to_owned();
    let delay = matches.get_one::<u64>("delay").unwrap_or(&120).to_owned();
    let num_bytes = matches.get_one::<u8>("num-bytes").unwrap_or(&4).to_owned();
    let ttl = matches.get_one::<u8>("ttl").unwrap_or(&128).to_owned();

    if !output_path.exists() || !output_path.is_dir() {
        eprintln!(
            "{}",
            format!("Output path {} is invalid. Exiting", output_path.display()).red()
        );
        std::process::exit(1);
    }

    // Need to check as otherwise timer will de-sync
    if timeout >= delay * 1000 {
        eprintln!(
            "{}",
            "Delay must be greater than the timeout. Exiting"
                .to_string()
                .red()
        );
        std::process::exit(1);
    }

    let app_task = task::spawn(async move {
        let mut stdout = stdout();

        let mut interval = tokio::time::interval(Duration::from_secs(delay));
        let mut engine = Engine::new(
            addr.clone(),
            ttl,
            timeout,
            num_bytes,
            delay,
            output_path.clone(),
        )
        .await;
        let dt_fmt = format_description::parse(DT_FMT).unwrap();
        stdout.execute(cursor::Hide).unwrap();

        loop {
            // wait for timer
            interval.tick().await;
            let (time, result) = engine.ping().await;
            let last_ping_text: StyledContent<String> =
                generate_ping_text(num_bytes, ttl, &dt_fmt, time, result);
            let last_successful_text: StyledContent<String> =
                generate_last_success_text(&mut engine, &dt_fmt);
            let last_failed_text: StyledContent<String> =
                generate_last_failed_text(&engine, &dt_fmt);
            stdout
                .execute(terminal::Clear(terminal::ClearType::FromCursorDown))
                .unwrap();
            display_tui(
                &stdout,
                &addr,
                &output_path,
                delay,
                timeout,
                ttl,
                num_bytes,
                &last_successful_text,
                &last_failed_text,
                &last_ping_text,
            );
            stdout.flush().unwrap();
            stdout.execute(cursor::MoveUp(10)).unwrap();
        }
    });
    // Below is invoked upon the user pressing Ctrl+C
    signal::ctrl_c().await.expect("event listener failure");
    // Move cursor down to prevent overwriting old TUI
    stdout().execute(cursor::MoveDown(10)).unwrap();
    println!("{}", "\nExiting".blue().bold());
    stdout().execute(cursor::Show).unwrap();
    app_task.abort();
}

/// Create stylized text representing the last time a ping failed. Red is used to indicate a failed
/// ping and green represents no failed pings up to the current time.
fn generate_last_failed_text(engine: &Engine, dt_fmt: &Vec<FormatItem>) -> StyledContent<String> {
    if let Some(last_failed_time) = engine.get_possible_last_failed_time() {
        last_failed_time.format(&dt_fmt).unwrap().red()
    } else {
        "N/A".to_string().green()
    }
}

/// Create stylized text representing the last time a ping succeeded (and the latency of that ping).
/// Red indicates no successful pings up to the current time while green represents a successful ping.
fn generate_last_success_text(
    engine: &mut Engine,
    dt_fmt: &Vec<FormatItem>,
) -> StyledContent<String> {
    if let Some(last_successful_time) = engine.get_possible_last_successful_time() {
        format!(
            "{} ({}ms)",
            last_successful_time.format(&dt_fmt).unwrap(),
            engine.get_last_successful_latency()
        )
        .green()
    } else {
        "N/A".to_string().red()
    }
}

/// Create stylized text representing data about the last ping performed. The text is red if the ping
/// failed, and green otherwise.
fn generate_ping_text(
    num_bytes: u8,
    ttl: u8,
    dt_fmt: &Vec<FormatItem>,
    time: OffsetDateTime,
    result: PingApiOutput,
) -> StyledContent<String> {
    if let Ok(result) = result {
        format!(
            "[{}] Reply from {}: bytes={} time={}ms TTL={}",
            time.format(&dt_fmt).unwrap(),
            result.address,
            num_bytes,
            result.rtt,
            ttl
        )
        .green()
    } else {
        format!("[{}] Ping failed.", time.format(&dt_fmt).unwrap()).red()
    }
}

/// Display a simple TUI (Terminal User Interface) to the user with basic statistics of the app
/// state.
#[allow(clippy::too_many_arguments)] // This method helps code readability in main
fn display_tui(
    mut stdout: &Stdout,
    addr: &String,
    output_path: &Path,
    delay: u64,
    timeout: u64,
    ttl: u8,
    num_bytes: u8,
    last_successful_text: &StyledContent<String>,
    last_failed_text: &StyledContent<String>,
    last_ping_text: &StyledContent<String>,
) {
    writeln!(
        stdout,
        "{}Target:{} {addr}",
        Attribute::Bold,
        Attribute::Reset
    )
    .unwrap();
    writeln!(
        stdout,
        "{}Output path:{} {}",
        Attribute::Bold,
        Attribute::Reset,
        output_path.canonicalize().unwrap().display()
    )
    .unwrap();
    writeln!(
        stdout,
        "{}Delay:{} {delay}s, {}Timeout:{} {timeout}ms",
        Attribute::Bold,
        Attribute::Reset,
        Attribute::Bold,
        Attribute::Reset
    )
    .unwrap();
    writeln!(
        stdout,
        "{}Num. Bytes:{} {num_bytes}, {}TTL:{} {ttl}",
        Attribute::Bold,
        Attribute::Reset,
        Attribute::Bold,
        Attribute::Reset
    )
    .unwrap();
    writeln!(
        stdout,
        "\n{}Last successful ping:{} {last_successful_text}",
        Attribute::Bold,
        Attribute::Reset
    )
    .unwrap();
    writeln!(
        stdout,
        "{}Last failed ping:{} {last_failed_text}",
        Attribute::Bold,
        Attribute::Reset
    )
    .unwrap();
    writeln!(
        stdout,
        "\n{}Last Ping Status:{}",
        Attribute::Bold,
        Attribute::Reset
    )
    .unwrap();
    writeln!(stdout, "{last_ping_text}").unwrap();
}
