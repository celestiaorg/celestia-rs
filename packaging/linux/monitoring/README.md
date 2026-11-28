# Monitor a Linux node

This Grafana dashboard reads the Prometheus metrics included in celestia-rs.
Run the stack on the same Linux host as the node. It needs Docker Compose
and free loopback ports 9090 and 3000.

1. Add `--prometheus-listen-addr 127.0.0.1:26660` to the node's normal
   `celestia-rs start` command and restart it. Keep the default
   instrumentation namespace (`cometbft`). Check the endpoint with
   `curl -fsS http://127.0.0.1:26660/metrics`.
2. From a checkout of celestia-rs, start both services.

   ```sh
   docker compose -f packaging/linux/monitoring/compose.yaml up -d
   ```

3. Open [Celestia node overview](http://127.0.0.1:3000/d/celestia-rs-operator).
   On first login, use `admin` / `admin` and set a new password. For a remote
   host, forward the UI with `ssh -L 3000:127.0.0.1:3000 user@node`.

The endpoint status shows whether Prometheus can reach the node. Committed
height and block interval show whether consensus is advancing. Peers, mempool
backlog, transaction throughput and database sizes help explain changes in
block production and resource use. Rate panels need at least two scrapes.
Consensus panels populate once the node reports commits.

The last row shows signing lag and locally observed misses for a validator.
Full nodes have no local validator series. The miss counter restarts with the
process. Check the on-chain slashing window when assessing jail risk.

To monitor additional nodes, add reachable targets to the `celestia-rs` job in
[prometheus.yml](prometheus.yml) and restart the Prometheus service. Select a
target with the **Node** dropdown. In an existing Grafana installation,
provision a Prometheus datasource with UID `celestia-rs-prom` and import
[the dashboard](dashboards/node.json) with the same scrape job name.

To stop the stack, run `docker compose -f packaging/linux/monitoring/compose.yaml down`.
The named volumes retain Prometheus history and Grafana settings.
