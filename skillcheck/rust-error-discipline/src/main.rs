#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::io;
use std::num::{ParseIntError, TryFromIntError};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::Context;
use log::warn;

const DEFAULT_LIMIT: u32 = 10;

#[derive(Debug, Default)]
struct Config {
    port: u16,
    limit: Option<u32>,
}

#[derive(Debug)]
struct Item(u32);

impl FromStr for Item {
    type Err = ParseIntError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse().map(Item)
    }
}

#[derive(Debug)]
struct User;

#[derive(Debug)]
struct Order;

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("syntax error")]
    Syntax,
    #[error("offset overflows")]
    Overflow,
    #[error("length out of range")]
    Range(#[from] TryFromIntError),
    #[error("invalid number")]
    Number(#[from] ParseIntError),
}

fn load_config(_path: &Path) -> anyhow::Result<Config> {
    Ok(Config::default())
}

fn find_limit(config: &Config) -> anyhow::Result<Option<u32>> {
    Ok(config.limit)
}

fn parse_order(_text: &str) -> Result<Order, Error> {
    Err(Error::Syntax)
}

fn propagate(path: PathBuf, raw: &str, lines: &[&str]) -> anyhow::Result<(Config, u32, Vec<Item>)> {
    // Prefer.
    let config = load_config(&path)?;
    let limit: u32 = raw.parse()?;
    let items: Vec<Item> = lines.iter().map(|l| l.parse()).collect::<Result<_, _>>()?;
    Ok((config, limit, items))
}

fn specified_default(config: Config) -> anyhow::Result<u32> {
    Ok(find_limit(&config)?.unwrap_or(DEFAULT_LIMIT))
}

fn cleanup(tmp: PathBuf) {
    // Prefer.
    if let Err(e) = fs::remove_file(&tmp) {
        warn!("removing {}: {e}", tmp.display());
    }
}

fn best_effort(tx: std::sync::mpsc::Sender<Item>, item: Item) {
    let _ = tx.send(item);
}

fn input_and_bugs(input: &str, users: &HashMap<u32, User>, id: u32, text: &str) -> Result<(), Error> {
    // Prefer.
    let age: u8 = input.parse()?;
    let user = users.get(&id).expect("ids are only handed out after insert");
    let order = parse_order(text)?;
    let _prefix = text.get(..4);
    println!("{age} {user:?} {order:?}");
    Ok(())
}

fn numbers(data: &[u8], offset: u32) -> Result<u32, Error> {
    // Prefer.
    let len = u32::try_from(data.len())?;
    let end = offset.checked_add(len).ok_or(Error::Overflow)?;
    Ok(end)
}

fn context(path: PathBuf) -> anyhow::Result<String> {
    // Prefer.
    let text = fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    Ok(text)
}

// Prefer.
#[derive(Debug, thiserror::Error)]
#[error("reading {}", .path.display())]
struct ReadError { path: PathBuf, source: io::Error }

fn read(path: &Path) -> Result<String, ReadError> {
    fs::read_to_string(path).map_err(|source| ReadError { path: path.into(), source })
}

// Prefer.
struct App { config: Config }

impl App {
    fn port(&self) -> u16 {
        self.config.port
    }
}

fn main() {
    let err = read(Path::new("/nonexistent/config.toml")).unwrap_err();
    println!("{err}: {}", std::error::Error::source(&err).is_some());
}
