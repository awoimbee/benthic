//! Coverage for device discovery and the download plumbing.
//!
//! Real protocols need real hardware, so these tests check that enumeration
//! and opening are total and that failures surface as clean errors rather than
//! panics or undefined behaviour.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use benthic_divecomputer::{descriptors, download, scan, DeviceId, Transport};

fn any_descriptor() -> benthic_divecomputer::DeviceDescriptor {
    descriptors()
        .expect("enumerate descriptors")
        .into_iter()
        .next()
        .expect("at least one descriptor")
}

#[test]
fn scanning_every_transport_never_panics() {
    let descriptor = any_descriptor();
    for transport in Transport::ALL {
        // A missing backend, permission problem or empty bus must be reported
        // as a value, never a panic.
        let _ = scan(&descriptor, transport);
    }
}

#[test]
fn opening_a_missing_serial_device_is_an_error() {
    let descriptor = any_descriptor();
    let id = DeviceId::Serial("/dev/benthic-no-such-dive-computer".to_string());
    let result = download(
        &descriptor,
        &id,
        &[],
        Arc::new(AtomicBool::new(false)),
        |_event| {},
    );
    assert!(result.is_err());
}

#[test]
fn unknown_models_are_rejected() {
    // Downloading from an unknown model fails before touching any transport.
    let stray = benthic_divecomputer::DeviceDescriptor {
        vendor: "No Such".to_string(),
        product: "Vendor".to_string(),
        family: 0,
        model: 0,
        transports: 0,
    };
    let id = DeviceId::Serial("/dev/null".to_string());
    assert!(scan(&stray, Transport::Serial).is_err());
    assert!(download(
        &stray,
        &id,
        &[],
        Arc::new(AtomicBool::new(false)),
        |_event| {}
    )
    .is_err());
}

#[test]
fn device_id_reports_its_transport_and_address() {
    assert_eq!(
        DeviceId::Serial("/dev/ttyUSB0".into()).transport(),
        Transport::Serial
    );
    assert_eq!(
        DeviceId::Usb {
            vid: 0x1234,
            pid: 0xabcd
        }
        .address(),
        "1234:abcd"
    );
    assert_eq!(
        DeviceId::UsbHid {
            vid: 0x0001,
            pid: 0x0002
        }
        .transport(),
        Transport::UsbHid
    );
}
