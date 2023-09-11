use firewood::codec::{decode_int, decode_str, encode_int, encode_str};

#[test]
fn test_bincode() {
    let val: i64 = 11;
    let encoded = encode_int(val).unwrap();
    println!("encoded: {:?}", encoded);

    let decoded: i64 = decode_int(&encoded[..]).unwrap();
    assert_eq!(val, decoded);

    let val = "hello world";
    let encoded = encode_str(val.as_bytes()).unwrap();
    println!("encoded: {:?}", encoded);

    let decoded: String = decode_str(&encoded[..]).unwrap();
    assert_eq!(val, decoded);
}
