use anyhow::{Context, Result};
use tun::AbstractDevice;

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub struct TunConfig {
    pub name: Option<String>,
    pub mtu: u32,
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
    #[allow(dead_code)]
    pub setup_addresses: bool,
}

pub fn create_tun(cfg: &TunConfig) -> Result<tun::Device> {
    let mut tun_cfg = tun::Configuration::default();

    tun_cfg.layer(tun::Layer::L3);

    if let Some(ref name) = cfg.name {
        tun_cfg.tun_name(name);
    }

    #[cfg(target_os = "linux")]
    tun_cfg.platform_config(|p| {
        p.ensure_root_privileges(true);
    });

    let dev = tun::create(&tun_cfg).context("failed to create TUN device")?;

    log::info!(
        "TUN device created: {}",
        dev.tun_name().unwrap_or_else(|_| "unknown".into())
    );
    Ok(dev)
}

#[cfg(target_os = "linux")]
pub async fn configure_tun(cfg: &TunConfig, dev: &tun::Device) -> Result<()> {
    use futures::stream::TryStreamExt;

    let tun_name = dev
        .tun_name()
        .context("failed to get TUN device name")?
        .clone();

    let (connection, handle, _) =
        rtnetlink::new_connection().context("failed to create netlink connection")?;
    tokio::spawn(connection);

    let mut links = handle.link().get().match_name(tun_name.clone()).execute();
    let link = links
        .try_next()
        .await
        .context("failed to query link")?
        .context("TUN device not found via netlink")?;
    let link_index = link.header.index;

    handle
        .link()
        .set(
            rtnetlink::LinkUnspec::new_with_index(link_index)
                .mtu(cfg.mtu)
                .build(),
        )
        .execute()
        .await
        .context("failed to set MTU")?;
    log::info!("MTU set to {}", cfg.mtu);

    if let Some(ref ipv4) = cfg.ipv4 {
        let addr: std::net::Ipv4Addr = ipv4.parse().context("invalid IPv4 address in config")?;
        handle
            .address()
            .add(link_index, std::net::IpAddr::V4(addr), 32)
            .execute()
            .await
            .context("failed to add IPv4 address")?;
        log::info!("IPv4 address {addr}/32 added");
    }

    if let Some(ref ipv6) = cfg.ipv6 {
        let addr: std::net::Ipv6Addr = ipv6.parse().context("invalid IPv6 address in config")?;
        handle
            .address()
            .add(link_index, std::net::IpAddr::V6(addr), 128)
            .execute()
            .await
            .context("failed to add IPv6 address")?;
        log::info!("IPv6 address {addr}/128 added");
    }

    handle
        .link()
        .set(
            rtnetlink::LinkUnspec::new_with_index(link_index)
                .up()
                .build(),
        )
        .execute()
        .await
        .context("failed to bring link up")?;
    log::info!("Link {tun_name} is UP");

    Ok(())
}

#[cfg(not(target_os = "linux"))]
#[allow(clippy::unused_async)]
pub async fn configure_tun(_cfg: &TunConfig, _dev: &tun::Device) -> Result<()> {
    anyhow::bail!("netlink TUN configuration is only supported on Linux; pass --no-iproute2")
}

// The one place the device is created and then configured over netlink as on
// the router, which starts the client without --no-iproute2. Needs root and
// /dev/net/tun; skipped otherwise.
#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::net::UdpSocket;

    fn sysfs(name: &str, file: &str) -> String {
        std::fs::read_to_string(format!("/sys/class/net/{name}/{file}"))
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    #[tokio::test]
    async fn the_device_gets_its_mtu_addresses_and_link_up_over_netlink() {
        let name = format!("usqn{}", std::process::id() % 100_000);
        let cfg = TunConfig {
            name: Some(name.clone()),
            mtu: 1280,
            ipv4: Some("172.31.254.2".to_string()),
            ipv6: Some("fd00:7573:7175::2".to_string()),
            setup_addresses: true,
        };
        let dev = match create_tun(&cfg) {
            Ok(dev) => dev,
            Err(error) => {
                eprintln!("Skipping TUN test (no permission): {error:#}");
                return;
            }
        };
        assert_eq!(dev.tun_name().expect("name"), name);
        assert!(
            UdpSocket::bind("172.31.254.2:0").is_err(),
            "the test address is already in use on this host"
        );

        configure_tun(&cfg, &dev).await.expect("configure");

        assert_eq!(sysfs(&name, "mtu"), "1280");
        let flags = sysfs(&name, "flags");
        let flags = u32::from_str_radix(flags.trim_start_matches("0x"), 16).expect("flags");
        assert_eq!(flags & 1, 1, "IFF_UP");

        // Only a local address can be bound.
        UdpSocket::bind("172.31.254.2:0").expect("IPv4 address is assigned");
        // A /32 adds no subnet route for the device.
        let routes = std::fs::read_to_string("/proc/net/route").expect("routes");
        assert!(
            !routes.lines().any(|line| line.starts_with(&name)),
            "{routes}"
        );

        // address, ifindex, prefix length (hex), scope, flags, name
        let inet6 = std::fs::read_to_string("/proc/net/if_inet6").expect("if_inet6");
        let ours: Vec<&str> = inet6
            .lines()
            .find(|line| line.starts_with("fd007573717500000000000000000002"))
            .expect("IPv6 address is assigned")
            .split_whitespace()
            .collect();
        assert_eq!(ours[2], "80", "prefix length 128");
        assert_eq!(ours[5], name);
    }
}
