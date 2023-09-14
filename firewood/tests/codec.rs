use firewood::codec::{decode_int, decode_str, encode_int, encode_str};

#[test]
fn test_bincode() {
    let val: i64 = 11;
    let encoded = encode_int(val).unwrap();
    let expected = vec![22];
    assert_eq!(encoded, expected);

    let decoded: i64 = decode_int(&encoded[..]).unwrap();
    assert_eq!(val, decoded);

    let val = "hello world";
    let encoded = encode_str(val.as_bytes()).unwrap();
    let expected = vec![22, 104, 101, 108, 108, 111, 32, 119, 111, 114, 108, 100];
    assert_eq!(encoded, expected);

    let decoded: String = decode_str(&encoded[..]).unwrap();
    assert_eq!(val, decoded);
}
