# pplot design

CLI clone of PingPlotter. Continuous traceroute with per-hop latency visualization.

## Decision summary

- Language: Rust
- Architecture: monolithic async (tokio), shared state via Arc<RwLock>
- TUI: ratatui + crossterm (interactive default, --report for non-interactive)
- Probes: ICMP + UDP + TCP
- Enrichment: reverse DNS only
- Persistence: none (in-memory)

## CLI interface

```
pplot <target> [options]
```

### Flags

| Flag | Default | Description |
|------|---------|-------------|
| `<target>` | required | One or more hostnames/IPs |
| `-i, --interval` | 1.0 | Seconds between probe rounds |
| `-m, --max-hops` | 30 | Maximum TTL |
| `-c, --count` | unlimited | Number of probe rounds |
| `-s, --size` | 64 | Packet size in bytes |
| `-t, --timeout` | 2.0 | Per-probe timeout (seconds) |
| `--icmp` | default | ICMP echo probes |
| `--udp` | - | UDP probes |
| `--tcp` | - | TCP SYN probes |
| `-p, --port` | 33434/80 | Target port for UDP/TCP |
| `--report` | - | Non-interactive output mode |
| `--csv` | - | CSV output (implies --report) |
| `--json` | - | JSON output (implies --report) |
| `-n, --no-dns` | - | Skip reverse DNS |
| `-4 / -6` | auto | Force IPv4/IPv6 |

## Module structure

```
src/
  main.rs              # CLI parsing (clap), mode dispatch
  lib.rs               # public API
  config.rs            # validated config from CLI args
  probe/
    mod.rs             # Probe trait + factory
    icmp.rs            # ICMP echo
    udp.rs             # UDP probes
    tcp.rs             # TCP SYN probes
    socket.rs          # raw socket helpers, privilege check
  trace/
    mod.rs             # TraceEngine orchestration
    state.rs           # TraceState, HopState, ProbeResult
    dns.rs             # async reverse DNS with cache
  tui/
    mod.rs             # TUI app loop
    widgets/
      hop_table.rs     # hop list with stats
      latency_chart.rs # per-hop sparkline over time
      timeline.rs      # time axis
      summary.rs       # header bar
      help.rs          # keybinding overlay
  report/
    mod.rs             # output dispatcher
    text.rs            # pretty table
    csv.rs             # CSV
    json.rs            # JSON
```

## Data model

```rust
struct TraceState {
    target: TargetInfo,
    hops: Vec<HopState>,
    round: u64,
    started_at: Instant,
}

struct HopState {
    ttl: u8,
    addr: Option<IpAddr>,
    hostname: Option<String>,
    samples: VecDeque<ProbeResult>,
    stats: HopStats,
}

struct HopStats {
    sent: u64,
    received: u64,
    lost: u64,
    loss_pct: f64,
    last_rtt: Option<Duration>,
    min_rtt: Option<Duration>,
    max_rtt: Option<Duration>,
    avg_rtt: f64,
    jitter: f64,
}

struct ProbeResult {
    seq: u64,
    rtt: Option<Duration>,
    timestamp: Instant,
}
```

## Async task topology

```
tokio::spawn(trace_engine)    # probes each interval, writes TraceState
tokio::spawn(dns_resolver)    # resolves new hop IPs in background
tokio::spawn(tui_loop)        # reads TraceState at 4-10fps
```

Shared via Arc<RwLock<TraceState>>. Write locks are brief (per probe round, per DNS result). TUI holds read lock per frame.

## TUI layout

```
+-  pplot - target (ip) --- round N --- HH:MM:SS -----------------------+
|                                                                        |
|  #  Host                   Loss%  Sent  Last   Avg   Best  Wrst  Std  |
|  1  192.168.1.1 (router)    0.0%   47   1.2   1.1   0.8   2.3   0.3  |
|  2  10.0.0.1                0.0%   47   8.4   8.2   7.1   12.3  1.1  |
|  ...                                                                   |
|                                                                        |
|  -- Latency (selected hop) ------------------------------------------ |
|  20ms |          __     __                                             |
|  10ms | _--__---  -_--_  --__------__--_                               |
|   5ms |-                                ---                            |
|       +---------------------------------------------- time ->          |
|                                                                        |
|  [q]uit  [up/dn]select  [g]raph  [p]ause  [r]eset                    |
+------------------------------------------------------------------------+
```

Two panes: hop stats table (top), latency chart for selected hop (bottom). Arrow keys select hop. Adapts to terminal size; chart collapses when too small.

## Dependencies

| Crate | Purpose |
|-------|---------|
| tokio | async runtime |
| clap | CLI args |
| ratatui + crossterm | TUI |
| socket2 | raw sockets |
| pnet | packet construction/parsing |
| hickory-resolver | async DNS |

## Privilege handling

- Attempt raw socket on startup
- Fail with clear message if unprivileged: suggest sudo (macOS) or cap_net_raw (Linux)
- UDP/TCP can fall back to unprivileged sockets where possible

## Report mode

Non-interactive table after --count rounds or Ctrl+C. Same stats as TUI hop table. --csv and --json for machine consumption.

## Error handling

- Non-responding hop: show `???`, track loss
- Route change: detect new IP at TTL, update display
- Target unreachable: show ICMP type, keep probing
- Terminal too small: table-only mode
- Ctrl+C: graceful shutdown, print summary
- Multiple targets: separate TraceState per target, tab between in TUI
