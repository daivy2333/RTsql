use rtsql::network::pg_messages;

#[test]
fn test_authentication_ok_serialization() {
    let bytes = pg_messages::authentication_ok();

    // PostgreSQL AuthenticationOk format: 'R' + length(8) + code(0)
    assert_eq!(bytes.len(), 9);
    assert_eq!(bytes[0], b'R'); // Message type

    // Length (Int32 BE): 8
    assert_eq!(bytes[1..5], [0, 0, 0, 8]);

    // Auth code (Int32 BE): 0 (AuthenticationOk)
    assert_eq!(bytes[5..9], [0, 0, 0, 0]);
}