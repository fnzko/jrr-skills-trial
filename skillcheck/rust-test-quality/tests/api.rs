use rust_test_quality_check::User;

mod common;

#[test]
fn creates_user() {
    let resp = common::app().post("/users", r#"{"name":"alice","age":30}"#);
    assert_eq!(resp.status, 201);
    assert_eq!(resp.json::<User>(), User { name: "alice".into(), age: 30 });
}

#[test]
fn rejects_user_without_age() {
    let resp = common::app().post("/users", r#"{"name":"alice"}"#);
    common::assert_status(&resp, 400);
}
