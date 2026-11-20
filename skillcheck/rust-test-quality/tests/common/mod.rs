use rust_test_quality_check::User;
use serde::de::DeserializeOwned;

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub body: String,
}

impl Response {
    pub fn json<T: DeserializeOwned>(&self) -> T {
        serde_json::from_str(&self.body).unwrap()
    }
}

pub struct App;

impl App {
    pub fn post(&self, path: &str, body: &str) -> Response {
        match (path, serde_json::from_str::<User>(body)) {
            ("/users", Ok(_)) => Response { status: 201, body: body.to_owned() },
            ("/users", Err(_)) => Response { status: 400, body: String::new() },
            _ => Response { status: 404, body: String::new() },
        }
    }
}

pub fn app() -> App {
    App
}

#[track_caller]
pub fn assert_status(resp: &Response, want: u16) {
    assert_eq!(resp.status, want, "{resp:?}");
}
