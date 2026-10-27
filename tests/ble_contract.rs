use somaris::ble::{
    BatteryReading, BleDecodeError, SOMA_MANUFACTURER_ID, decode_ba71_voltage, parse_soma_payload,
};

fn bytes(hex_value: &str) -> Vec<u8> {
    hex::decode(hex_value).unwrap()
}

#[test]
fn parses_captured_soma_advertisements_into_closed_percent() {
    let closed = parse_soma_payload(&bytes("055c6464426564726f6f6d2052696768743020")).unwrap();
    assert_eq!(SOMA_MANUFACTURER_ID, 0x0370);
    assert_eq!(closed.advertised_name, "Bedroom Right0");
    assert_eq!(closed.current_closed_percent, 100);
    assert_eq!(closed.target_closed_percent, 100);

    let opening = parse_soma_payload(&bytes("05356300426564726f6f6d2052696768743020")).unwrap();
    assert_eq!(opening.advertised_name, "Bedroom Right0");
    assert_eq!(opening.current_closed_percent, 99);
    assert_eq!(opening.target_closed_percent, 0);
}

#[test]
fn rejects_unsupported_versions_and_malformed_advertisements() {
    assert!(matches!(
        parse_soma_payload(&bytes("065c6464426564726f6f6d")),
        Err(BleDecodeError::UnsupportedVersion(6))
    ));
    assert!(matches!(
        parse_soma_payload(&bytes("055c64")),
        Err(BleDecodeError::PayloadTooShort)
    ));
    assert!(matches!(
        parse_soma_payload(&bytes("055c6464")),
        Err(BleDecodeError::MissingAdvertisedName)
    ));
    assert!(matches!(
        parse_soma_payload(&bytes("055c6564426c696e64")),
        Err(BleDecodeError::PositionOutOfRange)
    ));
}

#[test]
fn decodes_ba71_as_little_endian_tens_of_millivolts() {
    assert_eq!(
        decode_ba71_voltage(&[0x8e, 0x01]).unwrap(),
        BatteryReading {
            voltage_mv: 3980,
            percent_estimate: 76,
        }
    );
    assert_eq!(
        decode_ba71_voltage(&[0x9f, 0x01]).unwrap().percent_estimate,
        100
    );
    assert_eq!(
        decode_ba71_voltage(&[0x68, 0x01]).unwrap().percent_estimate,
        0
    );
    assert_eq!(
        decode_ba71_voltage(&[0x00, 0x00]),
        Err(BleDecodeError::InvalidBatteryValue)
    );
    assert_eq!(
        decode_ba71_voltage(&[0x01]),
        Err(BleDecodeError::InvalidBatteryValue)
    );
}
