# latensee

A PingPlotter-style CLI. Continuous traceroute with per-hop latency visualization in your terminal.

`latensee` runs repeated traceroutes to one or more targets, tracks per-hop statistics (loss, jitter, min/avg/max RTT), and plots latency over time in a live TUI. The name is a pun on "latency" + "see".

```
+- latensee - example.com (93.184.216.34) --- round 47 --- 00:01:23 --------+
|                                                                            |
|  #  Host                   Loss%  Sent  Last   Avg   Best  Wrst  Std      |
|  1  192.168.1.1 (router)    0.0%   47   1.2   1.1   0.8   2.3   0.3      |
|  2  10.0.0.1                0.0%   47   8.4   8.2   7.1   12.3  1.1      |
|  3  172.16.0.1              2.1%   47  14.2  13.8  11.5   22.7  2.4      |
|  ...                                                                       |
|                                                                            |
|  -- Latency: hop 2 (10.0.0.1) ------------------------------------------ |
|  12ms |          __     __                                                 |
|   8ms | _--__---  -_--_  --__------__--_                                   |
|   5ms |-                                ---                                |
|       +---------------------------------------------- time ->              |
|                                                                            |
|  [q]uit  [up/dn]select  [p]ause  [h]elp  [Tab]target                     |
+----------------------------------------------------------------------------+
```

## Installation

From crates.io (when published):

```sh
cargo install latensee
```

From source:

```sh
git clone https://github.com/vvojciech/latensee.git
cd latensee
cargo build --release
# binary at ./target/release/latensee
```

## Usage

```sh
# Basic traceroute with live TUI
sudo latensee example.com

# Multiple targets, switch between them with Tab
sudo latensee 8.8.8.8 1.1.1.1 example.com

# Fast probing, 20 rounds, then exit
sudo latensee -i 0.5 -c 20 example.com

# TCP probes to port 443 (useful when ICMP is blocked)
sudo latensee --tcp -p 443 example.com

# Non-interactive report as JSON
sudo latensee --json -c 10 example.com
```

## CLI flags

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `<targets>` | | required | One or more hostnames or IP addresses |
| `--interval` | `-i` | `1.0` | Seconds between probe rounds |
| `--max-hops` | `-m` | `30` | Maximum TTL (hop limit) |
| `--count` | `-c` | unlimited | Number of probe rounds, then exit |
| `--size` | `-s` | `64` | Packet size in bytes (min 28) |
| `--timeout` | `-t` | `2.0` | Per-probe timeout in seconds |
| `--icmp` | | default | Use ICMP echo probes |
| `--udp` | | | Use UDP probes |
| `--tcp` | | | Use TCP SYN probes |
| `--port` | `-p` | 33434 (UDP), 80 (TCP) | Target port for UDP/TCP probes |
| `--report` | | | Non-interactive output, print stats and exit |
| `--csv` | | | CSV output (implies `--report`) |
| `--json` | | | JSON output (implies `--report`) |
| `--no-dns` | `-n` | | Skip reverse DNS lookups |
| | `-4` | | Force IPv4 |
| | `-6` | | Force IPv6 |

Protocol flags (`--icmp`, `--udp`, `--tcp`) are mutually exclusive. Only one at a time.

## TUI keybindings

| Key | Action |
|-----|--------|
| `q` / `Esc` | Quit |
| `Up` / `k` | Select previous hop |
| `Down` / `j` | Select next hop |
| `p` | Pause/resume probing display |
| `h` / `?` | Toggle help overlay |
| `Tab` | Switch to next target |
| `Shift+Tab` | Switch to previous target |
| `r` | Reset statistics (reserved) |
| `g` | Toggle graph view (reserved) |

The latency chart appears below the hop table when the terminal is tall enough (20+ rows). On smaller terminals, you get the table only.

## Probe types

**ICMP** (default): Sends ICMP Echo Request packets with increasing TTL. Works well on most networks. Requires root/sudo.

**UDP** (`--udp`): Sends UDP packets to high ports (default 33434). Some networks filter ICMP but pass UDP. Does not require raw sockets, so it can run without sudo on some systems.

**TCP** (`--tcp`): Sends TCP SYN packets (default port 80). Useful for tracing the path to a specific service, or when both ICMP and UDP are filtered. Requires root/sudo.

Pick ICMP first. Switch to TCP or UDP if you see timeouts on hops that you know are reachable.

## Output formats

Without `--report`, latensee runs a live TUI and prints a text summary on exit (after `q` or Ctrl+C).

With `--report`, it runs non-interactively for `--count` rounds (or until Ctrl+C) and prints results:

- **Text** (default): formatted table matching the TUI layout
- **CSV** (`--csv`): one row per hop, suitable for spreadsheets or piping
- **JSON** (`--json`): structured output for scripting

## Requirements

**Privileges**: ICMP and TCP probes need raw sockets.
- macOS: run with `sudo`
- Linux: run with `sudo`, or set capabilities: `sudo setcap cap_net_raw+ep $(which latensee)`
- UDP probes may work without elevated privileges

**Platforms**: macOS and Linux.

**Rust**: 1.70+ (2021 edition).

## License

MIT
