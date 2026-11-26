# vmexec: run commands on a Compute Engine VM without SSH

`vmexec` drives a VM in project `fibreda` (zone `us-central1-a`) from an
environment that has `gcloud` but no SSH client. Requests go in through
instance metadata, results come back through guest attributes. With `--gcs`
the VM also gets the bucket `gs://fibreda-celestia-rs-genesis` for files of
any size. It needs `gcloud`, `jq` and `git` locally; nothing on the VM but
its stock Ubuntu 24.04 image.

## Quick start

```sh
V=scripts/vmexec/vmexec
$V create rs-1 --purpose follow-genesis --machine-type n2-standard-16 --boot-gb 150 --hours 24 --gcs
$V exec rs-1 -- 'nproc; df -h /'          # prints output, exits with the script's status
$V exec rs-1 -f ./script.sh               # a local script (or -f - for stdin)
$V ship rs-1                               # check out this repo's HEAD at /opt/celestia-rs
$V build rs-1                              # cargo build --release --locked -p celestia-rs (~4 min on 16 vCPUs)
$V job rs-1 start follow --status-cmd 'grep -o "height=[0-9]*" log | tail -1' -- \
   /opt/celestia-rs/target/release/celestia-rs follow --home /root/f --chain-id celestia \
   --rpc https://celestia-archive-rpc.polkachu.com,https://rpc.celestia.pops.one \
   --trust-period-hours 100000 --stop-at-height 5000
$V job rs-1 status follow                  # published every --interval s (default 60); no round trip
$V job rs-1 logs follow 200                # last 200 log lines
$V job rs-1 stop follow
$V fetch rs-1 /root/f/data/x.json ./x.json
$V delete rs-1
```

`vmexec help` lists every command and option.

## What each command does

- `create` makes the VM with labels `owner=claude-genesis`, `purpose=<P>`,
  `vmexec=1`, a maximum run duration (`--hours`, default 24) after which
  Compute Engine deletes it, no SSH keys, and no service account unless
  `--gcs` (then `claude-code-runner@…` with the storage scope only). The
  startup script installs the agent (`vmexec-agent.service`), disables
  unattended upgrades and needrestart restarts, and mounts a disk with device
  name `data` at `/data` (`--data-gb N` creates one, `--attach-disk DISK`
  attaches an existing one).
- `exec` runs a bash script as root in `/var/lib/vmexec/runs/<id>/` under
  `timeout` (`--timeout`, default 3600 s; status 124 on timeout). You get
  the last ~180 KB of stdout+stderr and the exit status. Longer output stays
  in that directory (`out`), and on `--gcs` VMs is also uploaded to
  `gs://fibreda-celestia-rs-genesis/vmexec/<vm>/runs/<id>.out`. A request
  survives a client crash: `--detach` prints the id, `result VM ID` collects
  it. One round trip takes about 12 s.
- `push`/`fetch` copy files. On `--gcs` VMs they go through
  `gs://…/vmexec/<vm>/{push,fetch}/` and have no practical size limit;
  otherwise they go through metadata at about 10 KB/s. Scripts on `--gcs`
  VMs can call `vmx-gcs put FILE OBJECT` and `vmx-gcs get OBJECT FILE`
  (objects are bucket-relative, e.g. `checkpoints/mainnet/2371489/…`).
- `ship` makes the VM check out your local `HEAD`. The VM fetches pushed
  commits anonymously from `github.com/celestiaorg/celestia-rs` (verified);
  commits that no remote branch has go as a `git bundle` via `push`.
  Uncommitted changes are not shipped. Without `--gcs`, a bundle above a
  few MB is slow (10 KB/s): push the branch, or use a `--gcs` VM.
- `build` installs build-essential, clang, protobuf and rustup, then runs
  cargo in `/opt/celestia-rs` (`-- CARGO_ARGS` overrides the arguments).
- `job` manages long-running commands as real systemd services
  (`vmxjob-<name>`, survive reboots, log to
  `/var/lib/vmexec/jobs/<name>/log`). `--restart` restarts on failure.
  Every `--interval` seconds the agent publishes the unit state, the
  `--status-cmd` output (run in the job directory, so `log` is the job log)
  and the last 25 log lines as guest attribute `vmx/job-<name>`.
- `status` shows the agent heartbeat (load, memory, disks) and all jobs.
- `serial` shows the serial console (for boot problems).

To put the agent on an existing VM, set its `startup-script` metadata to
`lib.sh` + `agent.sh` (with a `#!/usr/bin/env bash` line first) and reboot
it.

## Protocol and limits

- Request: metadata key `vmx-req-<id>` = `vmx1 <timeout>\n` +
  base64(gzip(script)). One value may hold 256 KiB and all values 512 KiB,
  so the client removes each key once it has the result (at most two
  large requests at once). The agent long-polls the attribute directory
  (`?wait_for_change=true`).
- Result: guest attribute `vmx/<id>`: `key=value` lines (`state`, `rc`,
  `bytes`, `truncated`, `output`, `output_gs`), `--`, then base64(gzip(the
  output's end)). A VM's guest attributes share about 512 KiB (measured:
  writes beyond that fail with HTTP 500), so results are capped at 120 KB
  and deleted once the client removed the request. Small writes are not
  rate limited (30 in 3 s worked).
- Tests: `bash scripts/vmexec/test.sh` runs everything offline against a
  fake `gcloud` and the agent in `VMX_FAKE` mode.
