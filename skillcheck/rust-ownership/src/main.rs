#![allow(dead_code, unused_variables, unused_mut)]

use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::{Arc, LazyLock, Mutex};

use regex::Regex;
use tracing::debug;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct UserId(u64);

#[derive(Debug, Clone)]
struct User {
    id: UserId,
    name: String,
}

#[derive(Debug, Clone)]
struct Item {
    name: String,
}

#[derive(Debug, Clone)]
struct Order {
    total: u64,
    paid: bool,
}

#[derive(Debug, Clone)]
struct Job;

#[derive(Debug, Clone)]
struct Config;

mod split {
    use super::{Item, Job, Order};

    #[derive(Default)]
    pub struct Index {
        names: Vec<String>,
    }

    impl Index {
        pub fn add(&mut self, item: &Item) {
            self.names.push(item.name.clone());
        }
    }

    #[derive(Default)]
    pub struct Catalog {
        items: Vec<Item>,
        index: Index,
        jobs: Vec<Job>,
        pending: Vec<Order>,
        runs: usize,
    }

    fn add_to(index: &mut Index, item: &Item) {
        index.add(item);
    }

    impl Catalog {
        fn run(&mut self, _job: &Job) {
            self.runs += 1;
        }

        pub fn reindex(&mut self) -> Vec<Order> {
            // Prefer.
            for item in &self.items {
                self.index.add(item);
            }
            let jobs = std::mem::take(&mut self.jobs);
            for job in &jobs {
                self.run(job);
            }
            self.jobs = jobs;
            let batch = std::mem::take(&mut self.pending);
            batch
        }

        pub fn reindex_destructured(&mut self) {
            let Self { items, index, .. } = self;
            for item in items.iter() {
                add_to(index, item);
            }
        }
    }
}

mod signatures {
    use super::{Order, User};

    pub struct Team {
        name: String,
        users: Vec<User>,
    }

    // Prefer.
    impl Team {
        fn new(name: String) -> Self { Self { name, users: Vec::new() } }
        fn name(&self) -> &str { &self.name }
        fn user_names(&self) -> impl Iterator<Item = &str> { self.users.iter().map(|u| u.name.as_str()) }
    }
    fn total(orders: &[Order]) -> u64 { orders.iter().map(|o| o.total).sum() }

    pub fn exercise(orders: &[Order]) -> usize {
        let team = Team::new("core".to_string());
        team.name().len() + team.user_names().count() + total(orders) as usize
    }
}

struct Worker {
    id: usize,
    config: Arc<Config>,
}

fn lookups(
    roles: &HashMap<String, u8>,
    name: &str,
    email: &str,
    other: &str,
    orders: &[Order],
    id: usize,
    config: Arc<Config>,
) -> (bool, bool, usize, Worker) {
    // Prefer.
    let known = roles.contains_key(name);
    let same = email.eq_ignore_ascii_case(other);
    let paid = orders.iter().filter(|o| o.paid).count();
    let worker = Worker { id, config: Arc::clone(&config) };
    (known, same, paid, worker)
}

fn notify(_user: Option<&User>) {}

// Prefer.
static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+$").unwrap());
fn is_number(s: &str) -> bool { NUMBER.is_match(s) }

fn notify_all(users: Vec<User>, ids: Vec<UserId>) {
    let by_id: HashMap<UserId, &User> = users.iter().map(|u| (u.id, u)).collect();
    for id in &ids {
        notify(by_id.get(id).copied());
    }
}

const MAX_PREALLOC: usize = 4096;

fn render(order: &Order, line: &mut String) {
    line.push_str(if order.paid { "paid" } else { "open" });
}

fn buffers(orders: &[Order], mut out: impl Write, declared_len: usize, state: &Config) -> io::Result<Vec<Item>> {
    // Prefer.
    let mut line = String::new();
    for order in orders {
        line.clear();
        render(order, &mut line);
        out.write_all(line.as_bytes())?;
    }
    let mut items: Vec<Item> = Vec::with_capacity(declared_len.min(MAX_PREALLOC));
    debug!(?state);
    Ok(items)
}

#[derive(Default)]
struct State {
    users: Vec<User>,
    pending: Vec<Order>,
}

fn submit(_order: Order) -> io::Result<()> {
    Ok(())
}

fn locks(state: &Mutex<State>) -> io::Result<usize> {
    // Prefer.
    let n = state.lock().unwrap().users.len();
    let pending = std::mem::take(&mut state.lock().unwrap().pending);
    for order in pending {
        submit(order)?;
    }
    Ok(n)
}

fn guards_in_block_heads_are_held(state: &Mutex<State>) -> bool {
    let mut held_for = false;
    for _user in state.lock().unwrap().users.iter() {
        held_for = state.try_lock().is_err();
    }
    let mut held_if_let = false;
    if let Some(_user) = state.lock().unwrap().users.first() {
        held_if_let = state.try_lock().is_err();
    }
    let held_match = match state.lock().unwrap().users.len() {
        _ => state.try_lock().is_err(),
    };
    state.lock().unwrap().pending.push(Order { total: 1, paid: false });
    let mut held_while_let = false;
    while let Some(_order) = state.lock().unwrap().pending.pop() {
        held_while_let = state.try_lock().is_err();
    }
    let released_after_let = {
        let n = state.lock().unwrap().users.len();
        state.try_lock().is_ok()
    };
    println!("for={held_for} if_let={held_if_let} match={held_match} while_let={held_while_let} released_after_let={released_after_let}");
    held_for && held_if_let && held_match && held_while_let && released_after_let
}

fn main() -> io::Result<()> {
    let mut catalog = split::Catalog::default();
    let _ = catalog.reindex();
    catalog.reindex_destructured();
    let orders = vec![Order { total: 3, paid: true }];
    let _ = signatures::exercise(&orders);
    let roles = HashMap::from([("admin".to_string(), 1u8)]);
    let (known, same, paid, _worker) = lookups(&roles, "admin", "A@x.io", "a@x.io", &orders, 0, Arc::new(Config));
    assert!(known && same && paid == 1);
    let users = vec![User { id: UserId(1), name: "ann".into() }];
    notify_all(users.clone(), vec![UserId(1)]);
    assert!(is_number("42") && !is_number("4x"));
    let items = buffers(&orders, io::sink(), usize::MAX, &Config)?;
    assert!(items.capacity() <= MAX_PREALLOC);
    let state = Mutex::new(State { users, pending: orders });
    assert_eq!(locks(&state)?, 1);
    assert!(state.lock().unwrap().pending.is_empty());
    assert!(guards_in_block_heads_are_held(&state));
    Ok(())
}
