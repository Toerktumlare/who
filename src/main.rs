use std::{
    io::{self, Stdout},
    process,
    time::{Duration, Instant},
};

use chrono::{DateTime, Local};

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use dns::{message::Message, DeSerialize, Serialize};
use tokio::net::UdpSocket;
use validation::{check_length, check_token_length};

use crate::dns::Buffer;
use ratatui::{prelude::*, widgets::*};
mod dns;
mod validation;

const TOP_BLOCK_SIZE: u16 = 1;
const HEADER_BLOCK_SIZE: u16 = 5;
const QUESTION_BLOCK_SIZE: u16 = 2;
const MESSAGE_BLOCK_SIZE: u16 = 2;
const STAT_BLOCK_SIZE: u16 = 6;

const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Statistics {
    pub query_time: Duration,
    pub msg_sent: usize,
    pub msg_rcvd: usize,
    pub current_time: DateTime<Local>,
}

#[derive(Subcommand)]
pub enum Commands {
    #[command(long_about = "fetch text records")]
    Txt { domain: String },
    #[command(long_about = "fetch cname records")]
    Cname { domain: String },
    #[command(long_about = "fetch A (ipv4) records")]
    A { domain: String },
    #[command(long_about = "fetch AAAA (ipv6) records")]
    AAAA { domain: String },
    #[command(long_about = "fetch NS (name server) records")]
    NS { domain: String },
    #[command(long_about = "fetch MX records")]
    MX { domain: String },
    #[command(long_about = "fetch SOA records")]
    SOA { domain: String },
}

#[derive(Parser)]
#[command(
    author,
    version,
    about = format!("== Who are you? == v{}", VERSION),
    long_about = format!("== Who are you? == v{} ==\n\na simple dns client written in rust to perform the most common dns queries.", VERSION),
)]
struct Cli {
    #[arg(help = "the domain you are asking for")]
    domain: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(short, long = "raw-records")]
    raw: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let m = match &cli.command {
        Some(Commands::Txt { domain }) => Message::txt(valid(domain)),
        Some(Commands::Cname { domain }) => Message::cname(valid(domain)),
        Some(Commands::A { domain }) => Message::a(valid(domain)),
        Some(Commands::AAAA { domain }) => Message::aaaa(valid(domain)),
        Some(Commands::NS { domain }) => Message::ns(valid(domain)),
        Some(Commands::MX { domain }) => Message::mx(valid(domain)),
        Some(Commands::SOA { domain }) => Message::soa(valid(domain)),
        None => {
            if let Some(address) = &cli.domain {
                Message::a(valid(address))
            } else {
                eprintln!("You must supply a valid address as a first argument");
                process::exit(1);
            }
        }
    };

    let sock = UdpSocket::bind("0.0.0.0:8080")
        .await
        .context("could not bind")?;

    let m = m.serialize().context("Failed to serialize request")?;

    let mut buffer = [0; 1024];
    let start = Instant::now();
    let _len = sock.send_to(&m, "1.1.1.1:53").await?;
    let (msg_length, _) = sock.recv_from(&mut buffer).await?;
    let elapsed = start.elapsed();

    let mut buffer = Buffer {
        current: &buffer,
        source: &buffer,
    };

    let (_buffer, message) =
        Message::deserialize(&mut buffer).context("Failed to deserialize response")?;

    let stats = Statistics {
        query_time: elapsed,
        msg_sent: m.len(),
        msg_rcvd: msg_length,
        current_time: Local::now(),
    };
    if !cli.raw {
        let mut terminal = setup_terminal(message.header.qd_count, message.header.an_count)
            .context("setup failed")?;
        terminal.draw(|f| render_app(f, &message, &stats))?;
        disable_raw_mode().context("failed to disable raw mode")?;
        let _ = terminal.show_cursor().context("unable to show cursor");
    } else {
        for r in message.records {
            println!("{}", r);
        }
    }

    Ok(())
}

fn valid(address: &String) -> &str {
    match validate(address) {
        Ok(address) => address,
        Err(msg) => {
            eprintln!("{}", msg);
            process::exit(1);
        }
    }
}

fn validate(address: &String) -> Result<&str> {
    let length_result = check_length(address);
    if !length_result {
        return Err(anyhow!(format!(
            "Address: {}, exceededs maximum length of 255",
            &address
        )));
    }
    let (value, result) = check_token_length(address);
    if !result {
        return Err(anyhow!(format!(
            "Token: {}, exceededs maximum length of 63",
            value
        )));
    }

    Ok(value)
}

fn setup_terminal(qd_count: u16, an_count: u16) -> Result<Terminal<CrosstermBackend<Stdout>>> {
    let viewport_size = TOP_BLOCK_SIZE
        + HEADER_BLOCK_SIZE
        + QUESTION_BLOCK_SIZE
        + qd_count
        + MESSAGE_BLOCK_SIZE
        + an_count
        + STAT_BLOCK_SIZE;

    let stdout = io::stdout();
    enable_raw_mode().context("failed to enable raw mode")?;
    let terminal = Terminal::with_options(
        CrosstermBackend::new(stdout),
        TerminalOptions {
            viewport: Viewport::Inline(viewport_size),
        },
    )?;
    Ok(terminal)
}

fn render_app(frame: &mut Frame, message: &Message, stats: &Statistics) {
    let outer = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(vec![Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(frame.size());

    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![
            Constraint::Length(TOP_BLOCK_SIZE),
            Constraint::Length(HEADER_BLOCK_SIZE),
            Constraint::Length(QUESTION_BLOCK_SIZE + message.header.qd_count),
            Constraint::Length(MESSAGE_BLOCK_SIZE + message.header.an_count),
            Constraint::Length(STAT_BLOCK_SIZE),
        ])
        .split(outer[0]);

    let program_info = Line::from(vec![
        "== Who are you? ==".into(),
        " ".into(),
        format!("v{}", VERSION).into(),
        " == ".into(),
        message.question.qname.clone().into(),
        " == ".into(),
    ]);

    frame.render_widget(Paragraph::new(program_info).fg(Color::White), inner[0]);

    // Header
    frame.render_widget(
        Paragraph::new(format!("{}", message.header))
            .fg(Color::White)
            .block(
                Block::new()
                    .title("Header")
                    .borders(Borders::ALL)
                    .fg(Color::Green),
            ),
        inner[1],
    );

    // Question
    let row = Row::new(vec![
        Cell::from(message.question.qname.clone()),
        Cell::from(""),
        Cell::from(message.question.qclass.to_string()),
        Cell::from(message.question.qtype.to_string()),
    ])
    .fg(Color::White);

    let t = Table::new(vec![row])
        .block(
            Block::new()
                .title("Message")
                .borders(Borders::ALL)
                .fg(Color::Green),
        )
        .widths(&[
            Constraint::Percentage(30),
            Constraint::Percentage(10),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
        ]);

    frame.render_widget(t, inner[2]);

    // Records
    let record_rows = message.records.iter().map(|r| {
        let string_data = match &r.rdata {
            dns::record::RData::A(ip) => ip.to_string(),
            dns::record::RData::CNAME(cname) => cname.to_string(),
            dns::record::RData::TXT(txt) => txt.to_string(),
            dns::record::RData::AAAA(ip) => ip.to_string(),
            dns::record::RData::NS(ns) => ns.to_string(),
            dns::record::RData::MX {
                preference,
                exchange,
            } => format!("{preference} {exchange}"),
            dns::record::RData::SOA {
                mname,
                rname,
                serial,
                refresh,
                retry,
                expire,
                minimum,
            } => format!("{mname} {rname} {serial} {refresh} {retry} {expire} {minimum}"),
        };

        Row::new(vec![
            Cell::from(r.name.clone()),
            Cell::from(r.ttl.as_secs().to_string()),
            Cell::from(r.qclass.to_string()),
            Cell::from(r.qtype.to_string()),
            Cell::from(string_data),
        ])
        .fg(Color::White)
    });

    let record_table = Table::new(record_rows)
        .block(
            Block::new()
                .title("Records")
                .borders(Borders::ALL)
                .fg(Color::Green),
        )
        .widths(&[
            Constraint::Percentage(30),
            Constraint::Percentage(10),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
            Constraint::Percentage(40),
        ]);
    frame.render_widget(record_table, inner[3]);

    let query_time = Line::from(vec![
        "Query time:".into(),
        " ".into(),
        stats.query_time.as_millis().to_string().into(),
        " ".into(),
        "msec".into(),
    ]);

    let current_time = Line::from(vec![
        "When:".into(),
        " ".into(),
        stats
            .current_time
            .format("%Y-%m-%d %H:%M:%S")
            .to_string()
            .into(),
    ]);

    let message_sent = Line::from(vec![
        "Msg SENT:".into(),
        " ".into(),
        stats.msg_sent.to_string().into(),
        " ".into(),
        "bytes".into(),
    ]);

    let message_rcvd = Line::from(vec![
        "Msg RCVD:".into(),
        " ".into(),
        stats.msg_rcvd.to_string().into(),
        " ".into(),
        "bytes".into(),
    ]);

    let t = Paragraph::new(vec![query_time, current_time, message_sent, message_rcvd])
        .block(
            Block::new()
                .title("Statistics")
                .borders(Borders::ALL)
                .fg(Color::Green),
        )
        .fg(Color::White);
    frame.render_widget(t, inner[4]);
}
