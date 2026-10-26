//! Integration tests for tunnel MTU handling: writes of each size through a real
//! TUN device. The packet pipeline itself is unit-tested in
//! `src/pipeline_tests.rs` against the production code.
//!
//! **Requires root or CAP_NET_ADMIN** - tests are skipped otherwise.

use std::io::Write;
use std::os::unix::io::{AsRawFd, FromRawFd};

const IPV4_HEADER_LEN: usize = 20;
const IPV6_HEADER_LEN: usize = 40;

fn make_ipv4_packet(total_len: usize, ttl: u8, src: [u8; 4], dst: [u8; 4]) -> Vec<u8> {
    assert!(total_len >= IPV4_HEADER_LEN);
    let mut pkt = vec![0xAAu8; total_len];
    pkt[0] = 0x45;
    pkt[1] = 0x00;
    pkt[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
    pkt[4..6].copy_from_slice(&[0x00, 0x00]);
    pkt[6..8].copy_from_slice(&[0x40, 0x00]);
    pkt[8] = ttl;
    pkt[9] = 17;
    pkt[10..12].copy_from_slice(&[0x00, 0x00]);
    pkt[12..16].copy_from_slice(&src);
    pkt[16..20].copy_from_slice(&dst);

    // Compute IP header checksum
    let cksum = ipv4_checksum(&pkt[..IPV4_HEADER_LEN]);
    pkt[10..12].copy_from_slice(&cksum.to_be_bytes());
    pkt
}

fn make_ipv6_packet(total_len: usize, hop_limit: u8, src: [u8; 16], dst: [u8; 16]) -> Vec<u8> {
    assert!(total_len >= IPV6_HEADER_LEN);
    let mut pkt = vec![0xBBu8; total_len];
    pkt[0] = 0x60;
    pkt[1] = 0x00;
    pkt[2] = 0x00;
    pkt[3] = 0x00;
    let payload_len = (total_len - IPV6_HEADER_LEN) as u16;
    pkt[4..6].copy_from_slice(&payload_len.to_be_bytes());
    pkt[6] = 17;
    pkt[7] = hop_limit;
    pkt[8..24].copy_from_slice(&src);
    pkt[24..40].copy_from_slice(&dst);
    pkt
}

fn ipv4_checksum(header: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i < header.len() {
        if i == 10 {
            i += 2;
            continue;
        }
        let word = if i + 1 < header.len() {
            u16::from_be_bytes([header[i], header[i + 1]])
        } else {
            u16::from_be_bytes([header[i], 0])
        };
        sum += word as u32;
        i += 2;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

fn try_create_tun(name: &str, mtu: u16) -> Option<tun::Device> {
    use tun::AbstractDevice;

    let mut config = tun::Configuration::default();
    config.layer(tun::Layer::L3);
    config.tun_name(name);

    #[cfg(target_os = "linux")]
    config.platform_config(|p| {
        p.ensure_root_privileges(true);
    });

    let mut dev = match tun::create(&config) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Skipping TUN test (no permission): {e}");
            return None;
        }
    };

    if let Err(e) = dev.set_mtu(mtu) {
        eprintln!("Failed to set MTU: {e}");
        return None;
    }

    if let Err(e) = dev.enabled(true) {
        eprintln!("Failed to bring device UP: {e}");
        return None;
    }

    Some(dev)
}

// ---- TUN device tests ----

#[test]
fn tun_device_small_mtu_ipv4_write() {
    let dev = match try_create_tun("usqt0", 576) {
        Some(d) => d,
        None => return, // skip if no permissions
    };

    let fd = dev.as_raw_fd();
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };

    let pkt = make_ipv4_packet(100, 64, [10, 200, 0, 1], [10, 200, 0, 2]);
    let result = file.write(&pkt);
    assert!(result.is_ok(), "should be able to write small IPv4 to TUN");
    assert_eq!(result.unwrap(), 100);

    // Don't let File close the fd (owned by Device)
    std::mem::forget(file);
}

#[test]
fn tun_device_large_mtu_ipv4_write() {
    let dev = match try_create_tun("usqt1", 9000) {
        Some(d) => d,
        None => return,
    };

    let fd = dev.as_raw_fd();
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };

    let pkt = make_ipv4_packet(8000, 64, [10, 200, 0, 1], [10, 200, 0, 2]);
    let result = file.write(&pkt);
    assert!(result.is_ok(), "should be able to write jumbo IPv4 to TUN");
    assert_eq!(result.unwrap(), 8000);

    std::mem::forget(file);
}

#[test]
fn tun_device_small_mtu_ipv6_write() {
    let dev = match try_create_tun("usqt2", 1280) {
        Some(d) => d,
        None => return,
    };

    let fd = dev.as_raw_fd();
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };

    let src = [0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
    let dst = [0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2];
    let pkt = make_ipv6_packet(1280, 64, src, dst);
    let result = file.write(&pkt);
    assert!(
        result.is_ok(),
        "should be able to write 1280-byte IPv6 to TUN"
    );

    std::mem::forget(file);
}

#[test]
fn tun_device_jumbo_mtu_ipv6_write() {
    let dev = match try_create_tun("usqt3", 9000) {
        Some(d) => d,
        None => return,
    };

    let fd = dev.as_raw_fd();
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };

    let src = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
    let dst = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2];
    let pkt = make_ipv6_packet(8000, 64, src, dst);
    let result = file.write(&pkt);
    assert!(result.is_ok(), "should be able to write jumbo IPv6 to TUN");

    std::mem::forget(file);
}
