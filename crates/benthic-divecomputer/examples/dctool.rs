//! Minimal command-line dive-computer tool.
//!
//! ```text
//! dctool list
//! dctool scan <vendor> <product> <transport>
//! dctool download <vendor> <product> <transport> <address>
//! ```
//!
//! `<transport>` is one of `serial`, `usb`, `usbhid`, `bluetooth`. The address
//! is a device node for serial, `vid:pid` (hex) for USB, and a Bluetooth
//! address for `bluetooth`.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use benthic_divecomputer::{descriptors, download, scan, DeviceEvent, DeviceId, Transport};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("list") => {
            for descriptor in descriptors().expect("enumerate descriptors") {
                println!("{} {}", descriptor.vendor, descriptor.product);
            }
        }
        Some("scan") if args.len() == 4 => {
            let descriptor = find(&args[1], &args[2]);
            let transport = parse_transport(&args[3]);
            for device in scan(&descriptor, transport).expect("scan") {
                println!("{}\t{}", device.id.address(), device.label());
            }
        }
        Some("download") if args.len() == 5 => {
            let descriptor = find(&args[1], &args[2]);
            let transport = parse_transport(&args[3]);
            let id = device_id(transport, &args[4]);
            let cancel = Arc::new(AtomicBool::new(false));
            let result = download(&descriptor, &id, &[], cancel, |event| match event {
                DeviceEvent::Progress { current, maximum } => {
                    eprintln!("progress {current}/{maximum}");
                }
                other => eprintln!("{other:?}"),
            })
            .expect("download");
            println!("downloaded {} dive(s)", result.dives.len());
            for (index, downloaded) in result.dives.iter().enumerate() {
                let computer = downloaded.dive.primary_computer();
                println!(
                    "  #{} when={} max_depth={:?} samples={}",
                    index + 1,
                    downloaded.dive.when,
                    downloaded.dive.max_depth(),
                    computer.map(|c| c.samples.len()).unwrap_or(0),
                );
            }
        }
        _ => {
            eprintln!(
                "usage:\n  dctool list\n  dctool scan <vendor> <product> <transport>\n  \
                 dctool download <vendor> <product> <transport> <address>"
            );
            std::process::exit(2);
        }
    }
}

fn find(vendor: &str, product: &str) -> benthic_divecomputer::DeviceDescriptor {
    descriptors()
        .expect("enumerate descriptors")
        .into_iter()
        .find(|d| d.vendor == vendor && d.product == product)
        .unwrap_or_else(|| panic!("unknown device: {vendor} {product}"))
}

fn parse_transport(name: &str) -> Transport {
    match name {
        "serial" => Transport::Serial,
        "usb" => Transport::Usb,
        "usbhid" => Transport::UsbHid,
        "bluetooth" | "bt" => Transport::Bluetooth,
        other => panic!("unknown transport: {other}"),
    }
}

fn device_id(transport: Transport, address: &str) -> DeviceId {
    match transport {
        Transport::Serial => DeviceId::Serial(address.to_string()),
        Transport::Usb | Transport::UsbHid => {
            let (vid, pid) = address.split_once(':').expect("expected vid:pid");
            let vid = u16::from_str_radix(vid, 16).expect("hex vid");
            let pid = u16::from_str_radix(pid, 16).expect("hex pid");
            match transport {
                Transport::Usb => DeviceId::Usb { vid, pid },
                _ => DeviceId::UsbHid { vid, pid },
            }
        }
        Transport::Bluetooth => {
            let value = address
                .split(':')
                .filter_map(|byte| u8::from_str_radix(byte, 16).ok())
                .fold(0u64, |acc, byte| (acc << 8) | u64::from(byte));
            DeviceId::Bluetooth {
                address: value,
                port: 0,
                name: address.to_string(),
            }
        }
        other => panic!("unsupported transport: {other:?}"),
    }
}
