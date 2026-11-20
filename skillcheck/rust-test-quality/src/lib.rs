use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct User {
    pub name: String,
    pub age: u8,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FindError {
    NotFound,
}

pub struct Store {
    pub users: Vec<User>,
}

pub fn list_users(store: &Store) -> Vec<User> {
    store.users.clone()
}

pub fn find_user(store: &Store, name: &str) -> Result<User, FindError> {
    store
        .users
        .iter()
        .find(|user| user.name == name)
        .cloned()
        .ok_or(FindError::NotFound)
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Config {
    pub data_dir: PathBuf,
    pub retries: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self { data_dir: PathBuf::from("data"), retries: 3 }
    }
}

pub fn load_config(path: &Path) -> io::Result<Config> {
    let text = fs::read_to_string(path)?;
    serde_json::from_str(&text).map_err(io::Error::other)
}

pub struct Item {
    pub price_cents: u64,
    pub quantity: u64,
}

pub fn item(price_cents: u64, quantity: u64) -> Item {
    Item { price_cents, quantity }
}

pub struct Order {
    pub items: Vec<Item>,
}

impl Order {
    pub fn total_cents(&self) -> u64 {
        self.items.iter().map(|i| i.price_cents * i.quantity).sum()
    }
}

pub struct Server {
    addr: SocketAddr,
    config: Config,
    ready: mpsc::Receiver<()>,
}

impl Server {
    pub fn start(listener: TcpListener, config: Config) -> Self {
        let addr = listener.local_addr().unwrap();
        let (ready_tx, ready) = mpsc::channel();
        thread::spawn(move || {
            ready_tx.send(()).unwrap();
            for stream in listener.incoming() {
                drop(stream);
            }
        });
        Self { addr, config, ready }
    }

    pub fn ready(&self) -> &mpsc::Receiver<()> {
        &self.ready
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn config(&self) -> &Config {
        &self.config
    }
}

pub trait Clock {
    fn now(&self) -> SystemTime;
}

#[derive(Clone)]
pub struct FakeClock(Arc<Mutex<SystemTime>>);

impl FakeClock {
    pub fn at(start: SystemTime) -> Self {
        Self(Arc::new(Mutex::new(start)))
    }

    pub fn advance(&self, by: Duration) {
        *self.0.lock().unwrap() += by;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> SystemTime {
        *self.0.lock().unwrap()
    }
}

pub struct Sessions<C> {
    clock: C,
    ttl: Duration,
    opened: BTreeMap<String, SystemTime>,
}

impl<C: Clock> Sessions<C> {
    pub fn new(clock: C, ttl: Duration) -> Self {
        Self { clock, ttl, opened: BTreeMap::new() }
    }

    pub fn open(&mut self, id: &str) {
        self.opened.insert(id.to_owned(), self.clock.now());
    }

    pub fn expired(&self) -> Vec<String> {
        let now = self.clock.now();
        self.opened
            .iter()
            .filter(|(_, opened)| **opened + self.ttl < now)
            .map(|(id, _)| id.clone())
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pending,
    Shipped,
}

#[derive(Debug, PartialEq, Eq)]
pub struct UnknownStatus(pub String);

impl FromStr for Status {
    type Err = UnknownStatus;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "shipped" => Ok(Self::Shipped),
            other => Err(UnknownStatus(other.to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store {
            users: vec![
                User { name: "alice".into(), age: 30 },
                User { name: "bob".into(), age: 41 },
            ],
        }
    }

    #[test]
    fn assert_the_exact_result() {
        let store = store();

        let names: Vec<_> = list_users(&store).into_iter().map(|u| u.name).collect();
        assert_eq!(names, ["alice", "bob"]);
        assert_eq!(find_user(&store, "carol"), Err(FindError::NotFound));
    }

    #[test]
    fn no_fallback() {
        let store = store();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, r#"{"data_dir":"data","retries":5}"#).unwrap();

        let config = load_config(&path).unwrap();
        assert_eq!(config.retries, 5);

        let user = find_user(&store, "alice").unwrap();
        assert_eq!(user.age, 30);
    }

    #[test]
    fn expected_from_spec() {
        let order = Order { items: vec![item(250, 2), item(750, 1)] };
        assert_eq!(order.total_cents(), 1250);
    }

    #[test]
    fn whole_value() {
        let body = r#"{"name":"alice","age":30}"#.to_owned();

        let user: User = serde_json::from_str(&body).unwrap();
        assert_eq!(user, User { name: "alice".into(), age: 30 });
    }

    #[test]
    fn deterministic_and_isolated() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config { data_dir: dir.path().to_path_buf(), retries: 5 };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = Server::start(listener, config);
        server.ready().recv_timeout(Duration::from_secs(5)).unwrap();

        assert_eq!(server.addr(), addr);
        assert_eq!(server.config().retries, 5);
    }

    #[test]
    fn fake_clock() {
        let clock = FakeClock::at(SystemTime::UNIX_EPOCH);
        let mut sessions = Sessions::new(clock.clone(), Duration::from_secs(60));
        sessions.open("s1");
        clock.advance(Duration::from_secs(61));
        assert_eq!(sessions.expired(), ["s1"]);
    }

    #[test]
    fn parses_known_statuses() {
        for (input, want) in [("pending", Status::Pending), ("shipped", Status::Shipped)] {
            assert_eq!(input.parse::<Status>(), Ok(want), "input {input:?}");
        }
    }
}
