#![allow(dead_code, unused_variables)]

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io;
use std::path::Path;
use std::thread;

struct Item {
    price: u64,
    quantity: u64,
}

struct User {
    name: String,
    email: Option<String>,
}

struct Env {
    port: Option<u16>,
}

struct Config {
    port: u16,
}

impl Config {
    fn overridden_by(self, env: &Env) -> Self {
        Self { port: env.port.unwrap_or(self.port) }
    }
}

fn parse(text: &str) -> io::Result<Config> {
    let port = text.trim().parse().map_err(io::Error::other)?;
    Ok(Config { port })
}

fn send<T: ?Sized>(_message: &T) -> io::Result<()> {
    Ok(())
}

mod add_nothing {
    use super::*;

    // Prefer.
    fn cleanup(dir: &Path) -> io::Result<()> { fs::remove_dir_all(dir) }

    struct Cart {
        items: Vec<Item>,
    }

    impl Cart {
        fn is_empty(&self) -> bool { self.items.is_empty() }
    }
}

mod no_twin {
    use super::*;

    // Prefer.
    fn load_config(path: &Path, env: &Env) -> io::Result<Config> {
        Ok(parse(&fs::read_to_string(path)?)?.overridden_by(env))
    }
}

mod one_decision_one_place {
    const FREE_SHIPPING_FROM: u32 = 5000;

    struct Order {
        total: u32,
    }

    // Prefer.
    impl Order {
        fn shipping_cost(&self) -> u32 { if self.total >= FREE_SHIPPING_FROM { 0 } else { 499 } }
        fn ships_free(&self) -> bool { self.shipping_cost() == 0 }
    }
}

mod helpers {
    use super::User;

    fn send(_message: &str) {}

    // Prefer.
    fn greet(user: &User) { send(&format!("hello {}", user.name)); }
}

mod bind {
    use super::*;

    fn notify(_user: &User) {}

    fn find(id: u32) -> Option<User> {
        None
    }

    fn prefer(users: &HashMap<u32, User>, id: u32) {
        // Prefer.
        if let Some(user) = users.get(&id) {
            notify(user);
        }
    }

    fn early(id: u32) {
        let Some(user) = find(id) else { return };
        notify(&user);
    }
}

mod one_match {
    const MAX_ATTEMPTS: u32 = 3;

    fn again() {}
    fn give_up() {}
    fn done() {}

    fn prefer(status: u16, attempts: u32, premium: bool) {
        // Prefer.
        match status {
            429 | 503 if attempts < MAX_ATTEMPTS => again(),
            429 | 503 => give_up(),
            _ => done(),
        }

        let limit = if premium { 100 } else { 10 };
    }
}

mod adapters {
    use super::*;

    fn prefer(users: Vec<User>, items: Vec<Item>) -> io::Result<()> {
        // Prefer.
        let emails: Vec<String> = users.iter().filter_map(|user| user.email.clone()).collect();
        for item in &items {
            send(item)?;
        }
        Ok(())
    }

    fn parse_all(lines: &[&str]) -> Result<Vec<u32>, std::num::ParseIntError> {
        let parsed = lines.iter().map(|line| line.parse()).collect::<Result<Vec<_>, _>>()?;
        Ok(parsed)
    }

    fn run(job: u32) {}

    fn phases(jobs: &[u32]) {
        let handles: Vec<_> = jobs.iter().map(|&job| thread::spawn(move || run(job))).collect();
        for handle in handles {
            handle.join().unwrap();
        }
    }
}

mod signatures {
    use super::*;

    struct Token;
    #[derive(Debug)]
    struct ParseError;

    fn parse(input: &str) -> Result<Vec<Token>, ParseError> {
        if input.is_empty() { Err(ParseError) } else { Ok(vec![Token]) }
    }

    fn port() -> Result<u16, Box<dyn std::error::Error>> {
        let port = env::var("PORT")?; let port: u16 = port.parse()?;
        Ok(port)
    }
}

mod scaffolding {
    use super::Item;

    // Prefer.
    fn total(items: &[Item]) -> u64 {
        items.iter().map(|item| item.price * item.quantity).sum()
    }

    #[expect(clippy::cast_possible_truncation, reason = "len is checked against u16::MAX above")]
    fn narrow(len: usize) -> u16 { len as u16 }
}

mod avoid {
    use super::*;

    trait Cleanup {
        fn run(&self, dir: &Path) -> io::Result<()>;
    }
    struct DefaultCleanup;
    impl Cleanup for DefaultCleanup {
        fn run(&self, dir: &Path) -> io::Result<()> { fs::remove_dir_all(dir) }
    }

    fn load_config(path: &Path) -> io::Result<Config> {
        parse(&fs::read_to_string(path)?)
    }
    fn load_config_with_env(path: &Path, env: &Env) -> io::Result<Config> {
        Ok(parse(&fs::read_to_string(path)?)?.overridden_by(env))
    }

    struct Order {
        total: u32,
    }
    impl Order {
        fn ships_free(&self) -> bool { self.total >= 5000 }
        fn shipping_cost(&self) -> u32 { if self.total > 5000 { 0 } else { 499 } }
    }

    fn format_greeting_for_user(user: &User) -> String { format!("hello {}", user.name) }

    fn greet(user: &User) { send(&format_greeting_for_user(user)).unwrap(); }

    fn notify(_user: &User) {}
    const MAX_ATTEMPTS: u32 = 3;
    fn again() {}
    fn give_up() {}
    fn done() {}

    fn flow(users: &HashMap<u32, User>, id: u32, status: u16, attempts: u32, items: Vec<Item>) {
        if users.get(&id).is_some() {
            notify(users.get(&id).unwrap());
        }

        let retry = status == 429 || status == 503;
        if retry && attempts < MAX_ATTEMPTS {
            again()
        } else if retry {
            give_up()
        } else {
            done()
        }

        let users: Vec<&User> = users.values().collect();
        let mut emails = Vec::new();
        for user in &users {
            if let Some(email) = &user.email {
                emails.push(email.clone());
            }
        }
        items.iter().for_each(|item| send(item).unwrap());
    }

    #[allow(dead_code)]
    fn old_total(items: &[Item]) -> u64 { items.iter().map(|item| item.price).sum() }

    fn total(items: &[Item]) -> u64 {
        dbg!(items.len());
        items.iter().map(|item| item.price * item.quantity).sum()
    }
}

fn main() {}
