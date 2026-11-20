use std::collections::HashSet;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct UserId(u32);

struct User {
    id: UserId,
}

#[derive(Debug, Clone, Copy)]
struct Item {
    id: u32,
}

#[derive(Debug)]
enum Error {
    Unsupported(&'static str),
}

fn find(items: &mut [Item], wanted: u32) -> Result<usize, usize> {
    // binary_search_by_key needs ascending ids.
    items.sort_by_key(|item| item.id);
    let found = items.binary_search_by_key(&wanted, |item| item.id);
    found
}

fn seen_users(users: &[User]) -> HashSet<UserId> {
    // A set, because membership is checked on every request.
    let seen: HashSet<UserId> = users.iter().map(|user| user.id).collect();
    seen
}

pub struct Config {
    timeout: Option<Duration>,
}

impl Config {
    /// Idle time after which the session is closed.
    ///
    /// `None` never closes it.
    pub fn timeout(&self) -> Option<Duration> { self.timeout }
}

struct Archive;

impl Archive {
    // TODO(#412): search the archive once it has an index.
    fn search(&self, query: &str) -> Result<Vec<Item>, Error> {
        Err(Error::Unsupported("search"))
    }
}

struct Buffer {
    ptr: *const Item,
    len: usize,
}

impl Buffer {
    fn get(&self, i: usize) -> Option<&Item> {
        if i >= self.len {
            return None;
        }
        // SAFETY: `i < self.len` was checked above, and `self.ptr` points to `self.len` initialized items.
        Some(unsafe { &*self.ptr.add(i) })
    }
}

fn main() {
    let mut items = vec![Item { id: 2 }, Item { id: 1 }];
    println!("{:?}", find(&mut items, 2));
    let users = [User { id: UserId(1) }];
    println!("{:?}", seen_users(&users));
    let config = Config { timeout: Some(Duration::from_secs(30)) };
    println!("{:?}", config.timeout());
    println!("{:?}", Archive.search("x"));
    let buffer = Buffer { ptr: items.as_ptr(), len: items.len() };
    println!("{:?} {:?}", buffer.get(1), buffer.get(5));
}
