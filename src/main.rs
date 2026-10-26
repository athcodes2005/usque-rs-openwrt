#![warn(
    clippy::all,
    clippy::pedantic,
    clippy::nursery,
    clippy::cargo,
    rust_2018_idioms
)]
#![allow(
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    // Transitive dependency duplicates we cannot control:
    clippy::multiple_crate_versions,
    clippy::redundant_pub_crate,
    clippy::similar_names,
    clippy::fn_params_excessive_bools,
    clippy::large_futures,
    clippy::items_after_statements,
    clippy::too_many_lines,
    clippy::cargo_common_metadata
)]
//! usque-rs - MASQUE (CONNECT-IP) client for Cloudflare WARP.

mod config;
mod events;
mod icmp;
mod packet;
mod register;
mod tls;
mod tun_device;
mod tunnel;

#[cfg(test)]
mod pipeline_tests;

use anyhow::Result;
use clap::{Parser, Subcommand};
use events::Severity;
use std::path::Path;
use std::time::Duration;

#[derive(Parser)]
#[command(
    name = "usque-rs",
    about = "Unofficial Cloudflare WARP MASQUE client in Rust"
)]
struct Cli {
    #[arg(short, long, default_value = "config.json")]
    config: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Register a new client and enroll a device key
    Register {
        #[arg(short, long, default_value = "en_US")]
        locale: String,
        #[arg(short, long, default_value = "PC")]
        model: String,
        #[arg(short, long)]
        name: Option<String>,
        #[arg(long)]
        jwt: Option<String>,
        #[arg(short, long, default_value_t = false)]
        accept_tos: bool,
    },
    /// Expose WARP as a native TUN device
    #[command(name = "nativetun")]
    NativeTun {
        #[arg(short = 'P', long, default_value_t = 443)]
        connect_port: u16,
        #[arg(short = '6', long, default_value_t = false)]
        ipv6: bool,
        #[arg(short = 'F', long, default_value_t = false)]
        no_tunnel_ipv4: bool,
        #[arg(short = 'S', long, default_value_t = false)]
        no_tunnel_ipv6: bool,
        #[arg(short, long, default_value = "consumer-masque.cloudflareclient.com")]
        sni_address: String,
        #[arg(short, long, default_value_t = 30)]
        keepalive_period: u64,
        #[arg(short, long, default_value_t = 1280)]
        mtu: u32,
        /// QUIC congestion control algorithm
        #[arg(
            long,
            default_value = "cubic",
            value_parser = ["cubic", "bbr", "bbr2", "reno"]
        )]
        congestion_control: String,
        #[arg(short = 'I', long, default_value_t = false)]
        no_iproute2: bool,
        #[arg(short = 'n', long)]
        interface_name: Option<String>,
        /// Write this marker after CONNECT-IP is ready; remove it on disconnect
        #[arg(long)]
        ready_file: Option<String>,
        /// Pause reading the TUN device while this many datagrams wait to be
        /// sent, so the excess queues in the kernel instead (0 disables)
        #[arg(long, default_value_t = 0)]
        tun_backpressure: usize,
        /// Drop upload datagrams, CoDel-style, while they wait longer than
        /// this many milliseconds to be sent (0 disables)
        #[arg(long, value_name = "MS", default_value_t = 0)]
        upload_codel: u64,
        /// Receive buffer for the QUIC UDP socket, so download bursts are not
        /// dropped by the kernel (Linux only; 0 keeps the kernel default)
        #[arg(long, value_name = "BYTES", default_value_t = 4_194_304)]
        udp_recv_buffer: usize,
        /// Also send connection events (connecting, connected, why a session
        /// ended, fatal errors) to the system log
        #[arg(long, default_value_t = false)]
        syslog: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let cli = Cli::parse();

    match cli.command {
        Commands::Register {
            locale,
            model,
            name,
            jwt,
            accept_tos,
        } => cmd_register(&cli.config, &locale, &model, name, jwt, accept_tos).await,
        Commands::NativeTun {
            connect_port,
            ipv6,
            no_tunnel_ipv4,
            no_tunnel_ipv6,
            sni_address,
            keepalive_period,
            mtu,
            congestion_control,
            no_iproute2,
            interface_name,
            ready_file,
            tun_backpressure,
            upload_codel,
            udp_recv_buffer,
            syslog,
        } => {
            if syslog {
                match events::Syslog::new(Path::new(events::SYSLOG_SOCKET)) {
                    Ok(syslog) => events::enable_syslog(syslog),
                    Err(error) => {
                        events::event(Severity::Warning, &format!("syslog unavailable: {error}"));
                    }
                }
            }
            let result = cmd_nativetun(
                &cli.config,
                connect_port,
                ipv6,
                no_tunnel_ipv4,
                no_tunnel_ipv6,
                &sni_address,
                Duration::from_secs(keepalive_period),
                mtu,
                &congestion_control,
                no_iproute2,
                interface_name,
                ready_file,
                tun_backpressure,
                Duration::from_millis(upload_codel),
                udp_recv_buffer,
            )
            .await;
            // main reports the error on stderr itself; syslog needs a copy.
            if let Err(error) = &result {
                events::syslog(Severity::Warning, &format!("[fatal] {error:#}"));
            }
            result
        }
    }
}

async fn cmd_register(
    config_path: &str,
    locale: &str,
    model: &str,
    device_name: Option<String>,
    jwt: Option<String>,
    accept_tos: bool,
) -> Result<()> {
    if let Ok(existing) = config::Config::load(config_path) {
        let _ = existing;
        eprint!("Config already exists. Overwrite? (y/n): ");
        let mut response = String::new();
        std::io::stdin().read_line(&mut response)?;
        if response.trim() != "y" {
            log::info!("Aborted.");
            return Ok(());
        }
    }

    if !accept_tos {
        eprintln!(
            "You must accept the Terms of Service \
             (https://www.cloudflare.com/application/terms/) to register."
        );
        eprint!("Do you agree? (y/n): ");
        let mut response = String::new();
        std::io::stdin().read_line(&mut response)?;
        if response.trim() != "y" {
            anyhow::bail!("User did not accept TOS");
        }
    }

    log::info!("Registering with locale={locale} model={model}");
    let account_data = register::register(model, locale, jwt.as_deref()).await?;
    log::info!("Registration successful, enrolling device key...");

    let (priv_key_der, pub_key_der) = register::generate_ec_keypair()?;
    let updated = register::enroll_key(&account_data, &pub_key_der, device_name.as_deref()).await?;

    let cfg = config::Config::from_account_data(&updated, &account_data.token, &priv_key_der)?;
    cfg.save(config_path)?;
    log::info!("Config saved to {config_path}");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn cmd_nativetun(
    config_path: &str,
    connect_port: u16,
    use_ipv6: bool,
    no_tunnel_ipv4: bool,
    no_tunnel_ipv6: bool,
    sni: &str,
    keepalive_period: Duration,
    mtu: u32,
    congestion_control: &str,
    no_iproute2: bool,
    interface_name: Option<String>,
    ready_file: Option<String>,
    tun_backpressure: usize,
    upload_codel: Duration,
    udp_recv_buffer: usize,
) -> Result<()> {
    if keepalive_period.is_zero() {
        anyhow::bail!("keepalive period must be at least one second");
    }
    let cfg = config::Config::load(config_path)?;
    events::event(Severity::Info, &format!("Config loaded from {config_path}"));

    let endpoint_ip: std::net::IpAddr = if use_ipv6 {
        cfg.endpoint_v6.parse()?
    } else {
        cfg.endpoint_v4.parse()?
    };
    let endpoint = std::net::SocketAddr::new(endpoint_ip, connect_port);

    let tun_cfg = tun_device::TunConfig {
        name: interface_name,
        mtu,
        ipv4: if no_tunnel_ipv4 {
            None
        } else {
            Some(cfg.ipv4.clone())
        },
        ipv6: if no_tunnel_ipv6 {
            None
        } else {
            Some(cfg.ipv6.clone())
        },
        setup_addresses: !no_iproute2,
    };
    let tun_dev = tun_device::create_tun(&tun_cfg)?;

    if no_iproute2 {
        events::event(Severity::Info, "Skipping address setup (--no-iproute2)");
    } else {
        tun_device::configure_tun(&tun_cfg, &tun_dev).await?;
    }

    let tunnel_cfg = tunnel::TunnelConfig {
        endpoint,
        sni: sni.to_string(),
        keepalive_period,
        mtu,
        congestion_control: congestion_control.to_string(),
        ready_file,
        tun_backpressure,
        upload_codel,
        udp_recv_buffer,
    };

    tunnel::maintain_tunnel(&cfg, &tunnel_cfg, tun_dev).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn nativetun_lists_the_upload_codel_for_tunnel_sh() {
        // tunnel.sh passes --upload-codel only to a binary whose help lists it.
        let mut cli = Cli::command();
        let help = cli
            .find_subcommand_mut("nativetun")
            .expect("nativetun subcommand")
            .render_help()
            .to_string();
        assert!(help.contains("--upload-codel <MS>"), "{help}");
    }

    #[test]
    fn the_upload_codel_is_off_unless_asked_for() {
        let target = |args: &[&str]| match Cli::parse_from(args.iter().copied()).command {
            Commands::NativeTun { upload_codel, .. } => upload_codel,
            Commands::Register { .. } => unreachable!("parsed nativetun"),
        };
        assert_eq!(target(&["usque-rs", "nativetun"]), 0);
        assert_eq!(
            target(&["usque-rs", "nativetun", "--upload-codel", "20"]),
            20
        );
    }

    #[test]
    fn nativetun_lists_the_udp_recv_buffer_for_tunnel_sh() {
        let mut cli = Cli::command();
        let help = cli
            .find_subcommand_mut("nativetun")
            .expect("nativetun subcommand")
            .render_help()
            .to_string();
        assert!(help.contains("--udp-recv-buffer <BYTES>"), "{help}");
    }

    #[test]
    fn the_udp_recv_buffer_defaults_to_four_mebibytes() {
        let target = |args: &[&str]| match Cli::parse_from(args.iter().copied()).command {
            Commands::NativeTun {
                udp_recv_buffer, ..
            } => udp_recv_buffer,
            Commands::Register { .. } => unreachable!("parsed nativetun"),
        };
        assert_eq!(target(&["usque-rs", "nativetun"]), 4 * 1024 * 1024);
        assert_eq!(
            target(&["usque-rs", "nativetun", "--udp-recv-buffer", "0"]),
            0
        );
    }
}
