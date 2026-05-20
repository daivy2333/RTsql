use rtsql::network::{Request, Response};

#[test]
fn test_request_serialize_query() {
    let req = Request::Query {
        sql: "SELECT * FROM users".to_string(),
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("Query"));
    assert!(json.contains("SELECT * FROM users"));
}

#[test]
fn test_request_deserialize_insert() {
    let json = r#"{"Insert":{"sql":"INSERT INTO users VALUES (1)"}}"#;
    let req: Request = serde_json::from_str(json).unwrap();
    assert_eq!(req.sql(), Some("INSERT INTO users VALUES (1)"));
}

#[test]
fn test_response_serialize_affected_rows() {
    let resp = Response::AffectedRows { count: 5 };
    let json = serde_json::to_string(&resp).unwrap();
    assert!(json.contains("AffectedRows"));
    assert!(json.contains("5"));
}

#[test]
fn test_response_deserialize_error() {
    let json = r#"{"Error":{"message":"table not found"}}"#;
    let resp: Response = serde_json::from_str(json).unwrap();
    match resp {
        Response::Error { message } => assert_eq!(message, "table not found"),
        _ => panic!("Expected Error response"),
    }
}

#[test]
fn test_ping_pong_roundtrip() {
    let req = Request::Ping;
    let json = serde_json::to_string(&req).unwrap();
    let parsed: Request = serde_json::from_str(&json).unwrap();
    assert!(matches!(parsed, Request::Ping));

    let resp = Response::Pong;
    let json = serde_json::to_string(&resp).unwrap();
    let parsed: Response = serde_json::from_str(&json).unwrap();
    assert!(matches!(parsed, Response::Pong));
}
