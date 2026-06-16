# latensee

A PingPlotter-style CLI. Continuous traceroute with per-hop latency visualization in your terminal.

`latensee` runs repeated traceroutes to one or more targets, tracks per-hop statistics (loss, jitter, min/avg/max RTT), and plots latency over time in a live TUI. The name is a pun on "latency" + "see".

```
+- latensee ---------------------------------------------------------------+
|                                                                           |
|  > example.com (93.184.216.34)     r47  00:01:23  last 12.3  avg 11.8  0%|
|    cloudflare.com (104.16.132.229) r45  00:01:21  last  4.1  avg  4.0  0%|
|                                                                           |
|  #  Host                   Loss%  Sent Errs  Last   Avg   Best  Wrst StDev|
|  1  192.168.1.1 (router)    0.0%   47    -   1.2   1.1   0.8   2.3   0.3 |
|  2  10.0.0.1                0.0%   47    -   8.4   8.2   7.1  12.3   1.1 |
|  3  172.16.0.1              2.1%   47    -  14.2  13.8  11.5  22.7   2.4 |
|  ...                                                                      |
|                                                                           |
|  -- Latency: hop 2 (10.0.0.1) ----------------------------------------- |
|  12ms |          __     __                                                |
|   8ms | _--__---  -_--_  --__------__--_                                  |
|   5ms |-                                ---                               |
|       +---------------------------------------------- time ->             |
|                                                                           |
|  [q]uit  [up/dn]target  [j/k]hop  [p]ause  [h]elp  [a]dd  [d]elete     |
+---------------------------------------------------------------------------+
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

# Multiple targets, all visible in list, Up/Down to switch
sudo latensee 8.8.8.8 1.1.1.1 example.com

# Fast probing, 20 rounds, then exit
sudo latensee -i 0.5 -c 20 example.com

# TCP probes to port 443 (useful when ICMP is blocked)
sudo latensee --tcp -p 443 example.com

# Non-interactive report as JSON
sudo latensee --json -c 10 example.com

# Without sudo (auto-falls back to TCP connect, target RTT only)
latensee 8.8.8.8 -c 5

# Explicit unprivileged TCP connect to port 443
latensee --tcp-connect -p 443 example.com
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
| `--tcp-connect` | | | Use unprivileged TCP connect probes (no root needed, target RTT only) |
| `--port` | `-p` | 33434 (UDP), 80 (TCP) | Target port for UDP/TCP probes |
| `--report` | | | Non-interactive output, print stats and exit |
| `--csv` | | | CSV output (implies `--report`) |
| `--json` | | | JSON output (implies `--report`) |
| `--no-dns` | `-n` | | Skip reverse DNS lookups |
| | `-4` | | Force IPv4 (default resolves both A and AAAA) |
| | `-6` | | Force IPv6 |

Protocol flags (`--icmp`, `--udp`, `--tcp`, `--tcp-connect`) are mutually exclusive. Only one at a time.

## TUI keybindings

| Key | Action |
|-----|--------|
| `q` / `Esc` / `Ctrl+C` | Quit |
| `Up` / `Down` | Select target |
| `j` / `k` | Select next/previous hop |
| `p` | Pause/resume probing |
| `h` / `?` | Toggle help overlay |
| `a` | Add target (type hostname/IP, press Enter) |
| `d` / `x` | Remove active target |
| `r` | Reset statistics |
| `g` | Toggle latency chart |

All targets are visible in a list at the top of the screen. Use Up/Down arrows to select which target's hops to view, and `j`/`k` to navigate hops within that target. Press `a` to add a new target at runtime, `d` to remove the selected one (can't remove the last).

The latency chart appears below the hop table when the terminal is tall enough (20+ rows). On smaller terminals, you get the table only.

## Probe types

**ICMP** (default): Sends ICMP Echo Request packets with increasing TTL. Works well on most networks. Requires root/sudo.

**UDP** (`--udp`): Sends UDP packets to high ports (default 33434). Some networks filter ICMP but pass UDP. Does not require raw sockets, so it can run without sudo on some systems.

**TCP** (`--tcp`): Sends TCP SYN packets (default port 80). Useful for tracing the path to a specific service, or when both ICMP and UDP are filtered. Requires root/sudo.

**TCP Connect** (`--tcp-connect`): Unprivileged TCP connect probes (default port 80). Measures target RTT without intermediate hops. No root/sudo needed. This is the automatic fallback when running without privileges.

Pick ICMP first. Switch to TCP or UDP if you see timeouts on hops that you know are reachable.

## Output formats

Without `--report`, latensee runs a live TUI and prints a text summary on exit (after `q` or Ctrl+C).

With `--report`, it runs non-interactively for `--count` rounds (or until Ctrl+C) and prints results:

- **Text** (default): formatted table matching the TUI layout
- **CSV** (`--csv`): one row per hop, suitable for spreadsheets or piping
- **JSON** (`--json`): structured output for scripting

## Requirements

**Privileges**: ICMP and TCP SYN probes need raw sockets.
- macOS: run with `sudo`
- Linux: run with `sudo`, or set capabilities: `sudo setcap cap_net_raw+ep $(which latensee)`
- Without root, latensee auto-falls back to TCP connect mode (target RTT only, no intermediate hops)
- Use `--tcp-connect` explicitly to skip the privilege check

**Platforms**: macOS and Linux.

**Rust**: 2021 edition. See `Cargo.toml` for dependency requirements.

## License

MIT
