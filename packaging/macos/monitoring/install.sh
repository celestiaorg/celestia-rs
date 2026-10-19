#!/usr/bin/env bash
# Installs Prometheus, Grafana and node_exporter monitoring for the
# celestia-rs nodes on this Mac (packaging/macos/monitoring/README.md).
#
#   packaging/macos/monitoring/install.sh            # cron supervisor, no sudo
#   packaging/macos/monitoring/install.sh --system   # LaunchDaemons (sudo)
#
# Set ALERT_EMAIL=you@gmail.com on the first run to get alert emails
# (saved in ~/.celestia-rs/monitoring/alert-email; README.md, "Email alerts").
#
# Both modes start at boot without a login session. Without sudo, the
# launchd user domain is not reachable over SSH, so a crontab entry
# (@reboot and every minute) runs bin/supervise.sh, which restarts any
# service that is not running and runs node-metrics.sh. --system uses
# LaunchDaemons instead, as the nodes do, and removes the cron entry.
#
# Re-run after editing any file here: it is idempotent, restarts the
# services, and keeps the Prometheus data, the Grafana database and the
# admin password.
set -euo pipefail

system=0
case ${1:-} in --system) system=1 ;; "") ;; *) echo "usage: $0 [--system]" >&2; exit 2 ;; esac

here=$(cd "$(dirname "$0")" && pwd)
MON=$HOME/.celestia-rs/monitoring
BREW=$(brew --prefix)
LABEL=org.celestia.monitoring
CRON_TAG="# celestia-rs-monitoring"

for f in prometheus grafana node_exporter; do
  [[ -d $BREW/opt/$f ]] || { echo "missing $f: brew install prometheus grafana node_exporter" >&2; exit 1; }
done
command -v jq >/dev/null || { echo "missing jq" >&2; exit 1; }

# Grafana's public URL: the tailnet name when Tailscale is up.
dns=$(tailscale status --json 2>/dev/null | jq -r '.Self.DNSName // empty' | sed 's/\.$//') || true
root_url=${dns:+https://$dns/}
root_url=${root_url:-http://127.0.0.1:3000/}

mkdir -p "$MON"
if [[ -n ${ALERT_EMAIL:-} ]]; then printf '%s\n' "$ALERT_EMAIL" >"$MON/alert-email"; fi
alert_email=$(cat "$MON/alert-email" 2>/dev/null || true)

render() {
  sed -e "s|__MON__|$MON|g" -e "s|__HOME__|$HOME|g" -e "s|__ROOT_URL__|$root_url|g" \
    -e "s|__ALERT_EMAIL__|$alert_email|g" "$1" >"$2"
}

# --- files -------------------------------------------------------------------
mkdir -p "$MON"/{bin,logs,textfile,prometheus/data,launchd} \
  "$MON"/grafana/{data,plugins,dashboards,provisioning/datasources,provisioning/dashboards,provisioning/alerting,provisioning/plugins}
install -m 755 "$here/node-metrics.sh" "$here/../../check-apphash.sh" "$MON/bin/"
render "$here/nodes.conf" "$MON/nodes.conf"
cp "$here/prometheus.yml" "$here/rules.yml" "$MON/prometheus/"
render "$here/grafana/grafana.ini" "$MON/grafana/grafana.ini"
cp "$here/grafana/provisioning/datasources/prometheus.yml" "$MON/grafana/provisioning/datasources/"
render "$here/grafana/provisioning/dashboards/celestia-rs.yml" "$MON/grafana/provisioning/dashboards/celestia-rs.yml"
cp "$here"/grafana/dashboards/*.json "$MON/grafana/dashboards/"
if [[ -n $alert_email ]]; then
  render "$here/grafana/provisioning/alerting/celestia-rs.yml" "$MON/grafana/provisioning/alerting/celestia-rs.yml"
else
  rm -f "$MON/grafana/provisioning/alerting/celestia-rs.yml"
  echo "note: no alert emails; re-run with ALERT_EMAIL=you@gmail.com to enable them" >&2
fi
if [[ ! -s $MON/grafana/admin-password ]]; then
  (umask 077; openssl rand -base64 18 | tr -d '\n' >"$MON/grafana/admin-password")
fi
# Grafana reads the Gmail app password from here; the operator writes it.
[[ -f $MON/grafana/smtp-password ]] || (umask 077; : >"$MON/grafana/smtp-password")
"$BREW/opt/prometheus/bin/promtool" check config "$MON/prometheus/prometheus.yml" >/dev/null

# --- services ----------------------------------------------------------------
# Long-running services (name, then command) and periodic jobs.
daemons=(prometheus node-exporter grafana)
# shellcheck disable=SC2034 # read as ${!v} by cmd_of and the plist loop
{
  cmd_prometheus=("$BREW/opt/prometheus/bin/prometheus"
    "--config.file=$MON/prometheus/prometheus.yml" "--storage.tsdb.path=$MON/prometheus/data"
    --storage.tsdb.retention.time=90d --web.listen-address=127.0.0.1:9090)
  cmd_node_exporter=("$BREW/opt/node_exporter/bin/node_exporter"
    --web.listen-address=127.0.0.1:9100 "--collector.textfile.directory=$MON/textfile")
  cmd_grafana=("$BREW/opt/grafana/bin/grafana" server
    "--homepath=$BREW/opt/grafana/share/grafana" "--config=$MON/grafana/grafana.ini")
}
collect=("$MON/bin/node-metrics.sh" collect "$MON/nodes.conf" "$MON/textfile")
apphash=("$MON/bin/node-metrics.sh" apphash "$MON/nodes.conf" "$MON/textfile")
env_path="$BREW/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"

cmd_of() { local v="cmd_${1//-/_}[@]"; printf '%q ' "${!v}"; }

# Services are found by their exact command line, as supervise.sh does,
# so a stale PID can never hide a stopped service or a duplicate.
stop_supervised() {
  local d v pids
  for d in "${daemons[@]}"; do
    v="cmd_${d//-/_}[*]"
    pids=$(ps -axo pid=,args= | awk -v c="${!v}" '{ p = $1; sub(/^ *[0-9]+ /, ""); if ($0 == c) print p }')
    [[ -n $pids ]] || continue
    # shellcheck disable=SC2086 # one PID per word
    {
      kill $pids 2>/dev/null || true
      for _ in $(seq 1 30); do kill -0 $pids 2>/dev/null || break; sleep 1; done
    }
  done
  pkill -f "^$MON/grafana/plugins/" 2>/dev/null || true # orphaned plugin processes
}

set_cron() { # set_cron <lines>: replace our tagged crontab lines
  {
    crontab -l 2>/dev/null | grep -vF "$CRON_TAG" || true
    if [[ -n $1 ]]; then printf '%s\n' "$1"; fi
  } | crontab -
}

# plist <name> <interval-seconds, or 0 for KeepAlive> <program> [args...]
plist() {
  local name=$1 interval=$2 a; shift 2
  {
    cat <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL.$name</string>
  <key>UserName</key><string>$(id -un)</string>
  <key>ProgramArguments</key>
  <array>
EOF
    for a in "$@"; do echo "    <string>$a</string>"; done
    echo "  </array>"
    echo "  <key>RunAtLoad</key><true/>"
    if ((interval)); then
      echo "  <key>StartInterval</key><integer>$interval</integer>"
    else
      echo "  <key>KeepAlive</key><true/>"
      echo "  <key>ThrottleInterval</key><integer>30</integer>"
    fi
    cat <<EOF
  <key>WorkingDirectory</key><string>$MON</string>
  <key>ProcessType</key><string>Background</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>HOME</key><string>$HOME</string>
    <key>PATH</key><string>$env_path</string>
  </dict>
  <key>StandardOutPath</key><string>$MON/logs/$name.log</string>
  <key>StandardErrorPath</key><string>$MON/logs/$name.log</string>
</dict>
</plist>
EOF
  } >"$MON/launchd/$LABEL.$name.plist"
  plutil -lint "$MON/launchd/$LABEL.$name.plist" >/dev/null
}

if ((system)); then
  set_cron ""
  stop_supervised
  for d in "${daemons[@]}"; do v="cmd_${d//-/_}[@]"; plist "$d" 0 "${!v}"; done
  plist node-metrics 30 "${collect[@]}"
  plist apphash 300 "${apphash[@]}"
  for j in "${daemons[@]}" node-metrics apphash; do
    f=/Library/LaunchDaemons/$LABEL.$j.plist
    sudo launchctl bootout "system/$LABEL.$j" 2>/dev/null || true
    sudo install -m 644 -o root -g wheel "$MON/launchd/$LABEL.$j.plist" "$f"
    sudo launchctl bootstrap system "$f"
  done
  mode="LaunchDaemons $LABEL.*"
else
  {
    cat <<EOF
#!/bin/bash
# Generated by packaging/macos/monitoring/install.sh; run by cron every
# minute and at boot. Starts each service that is not running, then
# collects node metrics twice a minute and checks app hashes every 5 min
# (and at once when the last result is missing).
export HOME=$(printf %q "$HOME") PATH=$env_path
cd $(printf %q "$MON") || exit 1
start() { # start <name> <command...>, unless that exact command line runs
  local n=\$1; shift
  ps -axo args= | grep -qxF -- "\$*" && return
  nohup "\$@" >>"logs/\$n.log" 2>&1 </dev/null &
}
EOF
    for d in "${daemons[@]}"; do echo "start $d $(cmd_of "$d")"; done
    printf '%q ' "${collect[@]}"; echo
    echo "(sleep 30; $(printf '%q ' "${collect[@]}")) &"
    echo "f=textfile/celestia_rs_apphash.prom"
    echo "if [[ ! -f \$f ]] || (( \$(date +%s) - \$(stat -f %m \$f) >= 290 )); then $(printf '%q ' "${apphash[@]}") & fi"
  } >"$MON/bin/supervise.sh"
  chmod 755 "$MON/bin/supervise.sh"
  set_cron "" # so cron does not start a service while it restarts
  stop_supervised
  for j in "${daemons[@]}" node-metrics apphash; do
    [[ -f /Library/LaunchDaemons/$LABEL.$j.plist ]] && echo "warning: $LABEL.$j LaunchDaemon is installed too (remove it or use --system)" >&2
  done
  "$MON/bin/supervise.sh" >>"$MON/logs/supervise.log" 2>&1
  sup=$(printf %q "$MON/bin/supervise.sh")
  set_cron "@reboot $sup >>$MON/logs/supervise.log 2>&1 $CRON_TAG
* * * * * $sup >>$MON/logs/supervise.log 2>&1 $CRON_TAG"
  mode="crontab ($MON/bin/supervise.sh)"
fi

# Publish Grafana on the tailnet only (HTTPS with a Tailscale certificate).
# The serve config is kept by tailscaled and survives reboots.
[[ -n $dns ]] && tailscale serve --bg --https=443 http://127.0.0.1:3000 >/dev/null

[[ -n $alert_email && ! -s $MON/grafana/smtp-password ]] &&
  echo "note: alert emails to $alert_email need a Gmail app password in $MON/grafana/smtp-password (README.md)" >&2
cat <<EOF
Monitoring installed in $MON, supervised by $mode.
  Grafana:    $root_url  (anyone on the tailnet can view; admin password in $MON/grafana/admin-password)
  Prometheus: http://127.0.0.1:9090  (this Mac only)
EOF
