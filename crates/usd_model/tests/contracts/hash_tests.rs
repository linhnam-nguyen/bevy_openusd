use super::*;

#[test]
fn hex_round_trip_preserves_all_bytes() {
    let original = HashDigest::new([
        0x00, 0x01, 0x0f, 0x10, 0x2a, 0x55, 0x7f, 0x80, 0xa5, 0xff, 0x11, 0x22, 0x33, 0x44, 0x66,
        0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xf0, 0xfe, 0x12, 0x34, 0x56, 0x78, 0x9a,
        0xbc, 0xde,
    ]);

    let encoded = original.to_hex();
    let decoded = HashDigest::from_hex(&encoded).expect("encoded digest must parse");

    assert_eq!(encoded.len(), HashDigest::HEX_LEN);
    assert_eq!(decoded, original);
    assert_eq!(decoded.to_string(), encoded);
}

#[test]
fn hex_parser_accepts_uppercase() {
    let digest = HashDigest::from_hex(&"AB".repeat(HashDigest::BYTE_LEN))
        .expect("uppercase hexadecimal must parse");

    assert_eq!(digest.as_bytes(), &[0xab; HashDigest::BYTE_LEN]);
}

#[test]
fn hex_parser_reports_invalid_input() {
    assert_eq!(
        HashDigest::from_hex("00"),
        Err(HashDigestError::InvalidLength {
            expected: HashDigest::HEX_LEN,
            actual: 2,
        })
    );

    let mut invalid = "00".repeat(HashDigest::HEX_LEN / 2);
    invalid.replace_range(30..31, "g");
    assert_eq!(
        HashDigest::from_hex(&invalid),
        Err(HashDigestError::InvalidHexDigit {
            index: 30,
            byte: b'g',
        })
    );
}
