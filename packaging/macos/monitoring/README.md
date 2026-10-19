# Monitoring the celestia-rs nodes on a Mac

Prometheus, Grafana and node_exporter for the Mocha and mainnet nodes on
one Mac, with a provisioned **celestia-rs nodes** dashboard and alert
rules. Grafana is reachable from your other devices over Tailscale only.

```
celestia-rs (mocha)   :26660 ┐
celestia-rs (mainnet) :26760 ├─> Prometheus :9090 ─> Grafana :3000 ─> tailscale serve
node_exporter         :9100 ┘                                        https://<mac>.<tailnet>.ts.net
   └─ textfile: node-metrics.sh (process, /status, x/slashing, app hash)
```

All ports listen on 127.0.0.1. `tailscale serve` publishes Grafana on
the tailnet with HTTPS. Nothing is exposed on the LAN or the internet.

## Install

Needs Homebrew, Tailscale, `jq`, and both nodes with
`[instrumentation] prometheus = true` (operator guide §11.2).

```sh
brew install prometheus grafana node_exporter
packaging/macos/monitoring/install.sh            # no sudo
packaging/macos/monitoring/install.sh --system   # or: LaunchDaemons (sudo)
```

Both modes start at boot without a login session:

- **Default (no sudo):** a crontab entry (`@reboot` and every minute)
  runs `~/.celestia-rs/monitoring/bin/supervise.sh`. It restarts any
  service that is not running, and it runs `node-metrics.sh`. Use this
  mode over SSH, where the launchd user domain is not available.
- **`--system`:** five `org.celestia.monitoring.*` LaunchDaemons, like
  the nodes' own jobs. This mode removes the cron entry.

Open `https://<mac>.<tailnet>.ts.net` from any device on the tailnet
(`tailscale status --json | jq -r .Self.DNSName`). Viewing needs no
login. To edit, sign in as `admin` with the password in
`~/.celestia-rs/monitoring/grafana/admin-password`.

Re-run `install.sh` after you edit any file here. It restarts the
services and keeps the Prometheus data (90 days), the Grafana database
and the password. To add a node, add a line to `nodes.conf` and a target
to `prometheus.yml` with the same `network` label.

## What is on the dashboard

The `Network` variable selects mocha, mainnet or both.

- **Overview:** node up, role (full node / validating / jailed), sync,
  app hash against a Go RPC, height, last block age, peers, firing alerts.
- **Validator signing:** blocks behind signing, signed share over the
  time range, missed blocks in the x/slashing window, share of the jail
  budget used, voting power. A full node shows "No data" here. These
  panels fill in when the node's key becomes a validator.
- **Consensus, transactions and mempool, node process, host:** block rate
  and interval, missing voting power, rounds, peers, txs, mempool, RSS,
  CPU, DB sizes, and the Mac's CPU, memory, disk and network.

## Alerts

`rules.yml` defines the alerts from operator guide §11.9: node down,
height stalled, app hash mismatch, not signing, missed blocks, jail risk,
jailed, low peers, and low disk. Firing alerts show in the dashboard's
"Firing alerts" panel, in Grafana under Alerting > Alert rules, and at
`http://127.0.0.1:9090/alerts`. Notifications (Slack, email, phone) need a
Grafana contact point, which this package does not set up.

## Files

| File | Installed to (`~/.celestia-rs/monitoring/`) |
|---|---|
| `node-metrics.sh` | `bin/`; `collect` every 30 s, `apphash` every 5 min |
| `nodes.conf` | `nodes.conf`: network, home, local RPC, reference RPC |
| `prometheus.yml`, `rules.yml` | `prometheus/` |
| `grafana/` | `grafana/`: config, datasource, dashboard |

Logs are in `~/.celestia-rs/monitoring/logs/`. Test the collector with
`packaging/macos/monitoring/test-node-metrics.sh`.

## Uninstall

```sh
crontab -l | grep -v celestia-rs-monitoring | crontab -
pkill -f "$HOME/.celestia-rs/monitoring/"
for j in prometheus node-exporter grafana node-metrics apphash; do   # --system only
  sudo launchctl bootout system/org.celestia.monitoring.$j
  sudo rm /Library/LaunchDaemons/org.celestia.monitoring.$j.plist
done
tailscale serve --https=443 off
```
