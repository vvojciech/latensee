mod config;
mod probe;
mod report;
mod trace;
mod tui;

use clap::Parser;
use std::sync::{Arc, RwLock};
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

    // Raw sockets needed for ICMP and TCP, not UDP
    if config.protocol != config::ProbeProtocol::Udp {
        if let Err(e) = probe::socket::check_privileges() {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }

    // Resolve first target to IP address
    let target_str = &config.targets[0];
    let addr = resolve_target(target_str, &config.ip_version).await?;
    let target_info = trace::state::TargetInfo {
        hostname: target_str.clone(),
        addr,
    };

    let state = Arc::new(RwLock::new(trace::state::TraceState::new(
        target_info,
        config.max_hops,
    )));
    let cancel = CancellationToken::new();

    // Trace engine
    let engine = trace::TraceEngine::new(Arc::clone(&state), &config);
    let cancel_engine = cancel.clone();
    let engine_handle = tokio::spawn(async move {
        engine.run(cancel_engine).await;
    });

    // Background DNS resolver
    let dns_state = Arc::clone(&state);
    let no_dns = config.no_dns;
    let cancel_dns = cancel.clone();
    let dns_handle = tokio::spawn(async move {
        if let Ok(resolver) = trace::dns::DnsResolver::new().await {
            trace::dns::run_dns_resolver(dns_state, resolver, no_dns, cancel_dns).await;
        }
    });

    // Ctrl+C cancellation
    let cancel_ctrlc = cancel.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        cancel_ctrlc.cancel();
    });

    if config.report {
        // Non-interactive: wait for engine to finish, then output
        engine_handle.await?;
        cancel.cancel();
        dns_handle.await?;

        let state = state.read().unwrap();
        if config.csv {
            print!("{}", report::csv_report::format_csv(&state));
        } else if config.json {
            println!("{}", report::json::format_json(&state));
        } else {
            report::text::print_report(&state);
        }
    } else {
        // Interactive TUI
        let tui_state = Arc::clone(&state);
        let cancel_tui = cancel.clone();
        let tui_result = tui::run_tui(tui_state, cancel_tui).await;

        cancel.cancel();
        engine_handle.await?;
        dns_handle.await?;

        // Print summary after TUI exits
        let state = state.read().unwrap();
        report::text::print_report(&state);

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
