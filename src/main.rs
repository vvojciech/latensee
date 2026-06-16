mod config;
mod probe;
mod report;
mod trace;
mod tui;

use clap::Parser;
use parking_lot::RwLock;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = config::Args::parse();
    let config = match config::Config::from_args(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    };

    let has_privileges = probe::socket::check_privileges().is_ok();
    let config = if config.protocol == config::ProbeProtocol::TcpConnect {
        config
    } else if !has_privileges {
        eprintln!(
            "Warning: no raw socket privileges, falling back to TCP connect mode \
             (target RTT only, no intermediate hops)"
        );
        let port = if config.port == 0 { 80 } else { config.port };
        config::Config {
            protocol: config::ProbeProtocol::TcpConnect,
            port,
            max_hops: 1,
            ..config
        }
    } else {
        config
    };

    // Resolve all targets to IP addresses
    let mut states: Vec<Arc<RwLock<trace::state::TraceState>>> = Vec::new();
    for target_str in &config.targets {
        let addr = resolve_target(target_str, &config.ip_version).await?;
        let target_info = trace::state::TargetInfo {
            hostname: target_str.clone(),
            addr,
        };
        let state = Arc::new(RwLock::new(trace::state::TraceState::new(
            target_info,
            config.max_hops,
        )));
        states.push(state);
    }

    let cancel = CancellationToken::new();
    let paused = Arc::new(AtomicBool::new(false));

    // Spawn a trace engine per target
    let mut engine_handles: Vec<JoinHandle<()>> = Vec::new();
    for state in &states {
        let engine =
            trace::TraceEngine::new(Arc::clone(state), &config, Arc::clone(&paused));
        let cancel_engine = cancel.clone();
        engine_handles.push(tokio::spawn(async move {
            engine.run(cancel_engine).await;
        }));
    }

    // Spawn a DNS resolver per target
    let mut dns_handles: Vec<JoinHandle<()>> = Vec::new();
    for state in &states {
        let dns_state = Arc::clone(state);
        let no_dns = config.no_dns;
        let cancel_dns = cancel.clone();
        dns_handles.push(tokio::spawn(async move {
            if let Ok(resolver) = trace::dns::DnsResolver::new().await {
                trace::dns::run_dns_resolver(dns_state, resolver, no_dns, cancel_dns).await;
            }
        }));
    }

    // Ctrl+C cancellation
    let cancel_ctrlc = cancel.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        cancel_ctrlc.cancel();
    });

    if config.report {
        // Non-interactive: wait for all engines to finish, then output
        for handle in engine_handles {
            handle.await?;
        }
        cancel.cancel();
        for handle in dns_handles {
            handle.await?;
        }

        let multi = states.len() > 1;
        for (i, state) in states.iter().enumerate() {
            if multi && i > 0 {
                println!();
            }
            let state = state.read();
            if config.csv {
                print!("{}", report::csv_report::format_csv(&state));
            } else if config.json {
                println!("{}", report::json::format_json(&state));
            } else {
                report::text::print_report(&state);
            }
        }
    } else {
        // Interactive TUI
        let cancel_tui = cancel.clone();
        let tui_result = tui::run_tui(states.clone(), cancel_tui, paused).await;

        cancel.cancel();
        for handle in engine_handles {
            handle.await?;
        }
        for handle in dns_handles {
            handle.await?;
        }

        // Print summary after TUI exits
        for (i, state) in states.iter().enumerate() {
            if i > 0 {
                println!();
            }
            let state = state.read();
            report::text::print_report(&state);
        }

        tui_result?;
    }

    Ok(())
}

async fn resolve_target(
    target: &str,
    ip_version: &config::IpVersion,
) -> Result<std::net::IpAddr, Box<dyn std::error::Error>> {
    // Direct IP address - no DNS needed
    if let Ok(addr) = target.parse::<std::net::IpAddr>() {
        return Ok(addr);
    }

    // DNS lookup using same resolver pattern as trace::dns
    let resolver = hickory_resolver::Resolver::builder_tokio()?.build();

    match ip_version {
        config::IpVersion::V6 => {
            let response = resolver.ipv6_lookup(target).await?;
            let addr = response.iter().next().ok_or("No AAAA record found")?;
            Ok(std::net::IpAddr::V6(**addr))
        }
        _ => {
            let response = resolver.ipv4_lookup(target).await?;
            let addr = response.iter().next().ok_or("No A record found")?;
            Ok(std::net::IpAddr::V4(**addr))
        }
    }
}
