use clap::Parser;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProbeProtocol {
    #[default]
    Icmp,
    Udp,
    Tcp,
    TcpConnect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IpVersion {
    #[default]
    Auto,
    V4,
    V6,
}

/// Continuous traceroute with per-hop latency visualization.
#[derive(Parser, Debug)]
#[command(name = "latensee", version, about)]
pub struct Args {
    /// Target hostnames or IP addresses
    #[arg(required = true)]
    pub targets: Vec<String>,

    /// Seconds between probe rounds
    #[arg(short = 'i', long = "interval", default_value = "1.0")]
    pub interval: f64,

    /// Maximum TTL (hop limit)
    #[arg(short = 'm', long = "max-hops", default_value = "30")]
    pub max_hops: u8,

    /// Number of probe rounds (unlimited if omitted)
    #[arg(short = 'c', long = "count")]
    pub count: Option<u64>,

    /// Packet size in bytes
    #[arg(short = 's', long = "size", default_value = "64")]
    pub size: u16,

    /// Per-probe timeout in seconds
    #[arg(short = 't', long = "timeout", default_value = "2.0")]
    pub timeout: f64,

    /// Use ICMP echo probes
    #[arg(long = "icmp", group = "protocol")]
    pub icmp: bool,

    /// Use UDP probes
    #[arg(long = "udp", group = "protocol")]
    pub udp: bool,

    /// Use TCP SYN probes
    #[arg(long = "tcp", group = "protocol")]
    pub tcp: bool,

    /// Use unprivileged TCP connect probes (no root needed, target RTT only)
    #[arg(long = "tcp-connect", group = "protocol")]
    pub tcp_connect: bool,

    /// Target port for UDP/TCP probes
    #[arg(short = 'p', long = "port")]
    pub port: Option<u16>,

    /// Non-interactive report output
    #[arg(long = "report")]
    pub report: bool,

    /// CSV output (implies --report)
    #[arg(long = "csv")]
    pub csv: bool,

    /// JSON output (implies --report)
    #[arg(long = "json")]
    pub json: bool,

    /// Skip reverse DNS lookups
    #[arg(short = 'n', long = "no-dns")]
    pub no_dns: bool,

    /// Force IPv4
    #[arg(short = '4')]
    pub ipv4: bool,

    /// Force IPv6
    #[arg(short = '6')]
    pub ipv6: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub targets: Vec<String>,
    pub interval: f64,
    pub max_hops: u8,
    pub count: Option<u64>,
    pub size: u16,
    pub timeout: f64,
    pub protocol: ProbeProtocol,
    pub port: u16,
    pub report: bool,
    pub csv: bool,
    pub json: bool,
    pub no_dns: bool,
    pub ip_version: IpVersion,
}

impl Config {
    pub fn from_args(args: &Args) -> anyhow::Result<Self> {
        if args.interval <= 0.0 {
            anyhow::bail!("interval must be greater than 0");
        }
        if args.timeout <= 0.0 {
            anyhow::bail!("timeout must be greater than 0");
        }
        if args.size < 28 {
            anyhow::bail!("packet size must be at least 28 bytes");
        }

        let protocol = if args.tcp {
            ProbeProtocol::Tcp
        } else if args.udp {
            ProbeProtocol::Udp
        } else if args.tcp_connect {
            ProbeProtocol::TcpConnect
        } else {
            ProbeProtocol::Icmp
        };

        let port = args.port.unwrap_or(match protocol {
            ProbeProtocol::Icmp => 0,
            ProbeProtocol::Udp => 33434,
            ProbeProtocol::Tcp | ProbeProtocol::TcpConnect => 80,
        });

        let report = args.report || args.csv || args.json;

        let ip_version = if args.ipv4 {
            IpVersion::V4
        } else if args.ipv6 {
            IpVersion::V6
        } else {
            IpVersion::Auto
        };

        Ok(Config {
            targets: args.targets.clone(),
            interval: args.interval,
            max_hops: args.max_hops,
            count: args.count,
            size: args.size,
            timeout: args.timeout,
            protocol,
            port,
            report,
            csv: args.csv,
            json: args.json,
            no_dns: args.no_dns,
            ip_version,
        })
    }
}

pub async fn resolve_target(
    target: &str,
    ip_version: &IpVersion,
) -> anyhow::Result<std::net::IpAddr> {
    if let Ok(addr) = target.parse::<std::net::IpAddr>() {
        return Ok(addr);
    }

    let resolver = hickory_resolver::Resolver::builder_tokio()?.build();

    match ip_version {
        IpVersion::V6 => {
            let response = resolver.ipv6_lookup(target).await?;
            let addr = response
                .iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("No AAAA record found"))?;
            Ok(std::net::IpAddr::V6(**addr))
        }
        IpVersion::V4 => {
            let response = resolver.ipv4_lookup(target).await?;
            let addr = response
                .iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("No A record found"))?;
            Ok(std::net::IpAddr::V4(**addr))
        }
        IpVersion::Auto => {
            let response = resolver.lookup_ip(target).await?;
            let addr = response
                .iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("No A or AAAA record found"))?;
            Ok(addr)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Args {
        let mut full = vec!["latensee"];
        full.extend_from_slice(args);
        Args::parse_from(full)
    }

    #[test]
    fn default_args_parse_correctly() {
        let args = parse(&["example.com"]);
        let config = Config::from_args(&args).unwrap();

        assert_eq!(config.targets, vec!["example.com"]);
        assert_eq!(config.interval, 1.0);
        assert_eq!(config.max_hops, 30);
        assert_eq!(config.count, None);
        assert_eq!(config.size, 64);
        assert_eq!(config.timeout, 2.0);
        assert_eq!(config.protocol, ProbeProtocol::Icmp);
        assert_eq!(config.port, 0);
        assert!(!config.report);
        assert!(!config.csv);
        assert!(!config.json);
        assert!(!config.no_dns);
        assert_eq!(config.ip_version, IpVersion::Auto);
    }

    #[test]
    fn multiple_targets() {
        let args = parse(&["example.com", "8.8.8.8"]);
        let config = Config::from_args(&args).unwrap();
        assert_eq!(config.targets, vec!["example.com", "8.8.8.8"]);
    }

    #[test]
    fn protocol_udp() {
        let args = parse(&["example.com", "--udp"]);
        let config = Config::from_args(&args).unwrap();
        assert_eq!(config.protocol, ProbeProtocol::Udp);
    }

    #[test]
    fn protocol_tcp() {
        let args = parse(&["example.com", "--tcp"]);
        let config = Config::from_args(&args).unwrap();
        assert_eq!(config.protocol, ProbeProtocol::Tcp);
    }

    #[test]
    fn protocol_default_is_icmp() {
        let args = parse(&["example.com"]);
        let config = Config::from_args(&args).unwrap();
        assert_eq!(config.protocol, ProbeProtocol::Icmp);
    }

    #[test]
    fn csv_implies_report() {
        let args = parse(&["example.com", "--csv"]);
        let config = Config::from_args(&args).unwrap();
        assert!(config.report);
        assert!(config.csv);
    }

    #[test]
    fn json_implies_report() {
        let args = parse(&["example.com", "--json"]);
        let config = Config::from_args(&args).unwrap();
        assert!(config.report);
        assert!(config.json);
    }

    #[test]
    fn port_defaults_udp() {
        let args = parse(&["example.com", "--udp"]);
        let config = Config::from_args(&args).unwrap();
        assert_eq!(config.port, 33434);
    }

    #[test]
    fn port_defaults_tcp() {
        let args = parse(&["example.com", "--tcp"]);
        let config = Config::from_args(&args).unwrap();
        assert_eq!(config.port, 80);
    }

    #[test]
    fn port_defaults_icmp() {
        let args = parse(&["example.com"]);
        let config = Config::from_args(&args).unwrap();
        assert_eq!(config.port, 0);
    }

    #[test]
    fn explicit_port_overrides_default() {
        let args = parse(&["example.com", "--tcp", "-p", "443"]);
        let config = Config::from_args(&args).unwrap();
        assert_eq!(config.port, 443);
    }

    #[test]
    fn validation_rejects_zero_interval() {
        let args = parse(&["example.com", "-i", "0"]);
        let result = Config::from_args(&args);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("interval"));
    }

    #[test]
    fn validation_rejects_negative_interval() {
        let args = parse(&["example.com", "--interval=-0.5"]);
        let result = Config::from_args(&args);
        assert!(result.is_err());
    }

    #[test]
    fn validation_rejects_zero_timeout() {
        let args = parse(&["example.com", "-t", "0"]);
        let result = Config::from_args(&args);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timeout"));
    }

    #[test]
    fn validation_rejects_small_packet_size() {
        let args = parse(&["example.com", "-s", "27"]);
        let result = Config::from_args(&args);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("28"));
    }

    #[test]
    fn size_28_is_accepted() {
        let args = parse(&["example.com", "-s", "28"]);
        let config = Config::from_args(&args).unwrap();
        assert_eq!(config.size, 28);
    }

    #[test]
    fn ipv4_flag() {
        let args = parse(&["example.com", "-4"]);
        let config = Config::from_args(&args).unwrap();
        assert_eq!(config.ip_version, IpVersion::V4);
    }

    #[test]
    fn ipv6_flag() {
        let args = parse(&["example.com", "-6"]);
        let config = Config::from_args(&args).unwrap();
        assert_eq!(config.ip_version, IpVersion::V6);
    }

    #[test]
    fn no_dns_flag() {
        let args = parse(&["example.com", "-n"]);
        let config = Config::from_args(&args).unwrap();
        assert!(config.no_dns);
    }

    #[test]
    fn count_option() {
        let args = parse(&["example.com", "-c", "10"]);
        let config = Config::from_args(&args).unwrap();
        assert_eq!(config.count, Some(10));
    }

    #[test]
    fn protocol_tcp_connect() {
        let args = parse(&["example.com", "--tcp-connect"]);
        let config = Config::from_args(&args).unwrap();
        assert_eq!(config.protocol, ProbeProtocol::TcpConnect);
    }

    #[test]
    fn port_defaults_tcp_connect() {
        let args = parse(&["example.com", "--tcp-connect"]);
        let config = Config::from_args(&args).unwrap();
        assert_eq!(config.port, 80);
    }

    #[test]
    fn all_options_combined() {
        let args = parse(&[
            "example.com",
            "1.1.1.1",
            "-i", "0.5",
            "-m", "20",
            "-c", "100",
            "-s", "128",
            "-t", "3.0",
            "--tcp",
            "-p", "443",
            "--report",
            "-n",
            "-4",
        ]);
        let config = Config::from_args(&args).unwrap();
        assert_eq!(config.targets, vec!["example.com", "1.1.1.1"]);
        assert_eq!(config.interval, 0.5);
        assert_eq!(config.max_hops, 20);
        assert_eq!(config.count, Some(100));
        assert_eq!(config.size, 128);
        assert_eq!(config.timeout, 3.0);
        assert_eq!(config.protocol, ProbeProtocol::Tcp);
        assert_eq!(config.port, 443);
        assert!(config.report);
        assert!(config.no_dns);
        assert_eq!(config.ip_version, IpVersion::V4);
    }
}
