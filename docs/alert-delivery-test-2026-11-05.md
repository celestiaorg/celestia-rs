# Critical alert delivery test — 2026-11-05

The owner authorized a non-disruptive end-to-end alert test. The test
used the existing `AppHashMismatch` critical rule with a synthetic
`network="delivery-test"` time series in node-exporter's textfile
directory. Neither validator's metrics, process, RPC, stake or signing
state was changed. All times below are UTC on 2026-11-06 (Pacific
afternoon on Nov 5).

| Time | Observation |
|---|---|
| 01:01:22 | Synthetic textfile installed. Node-exporter exposed `celestia_rs_apphash_match{network="delivery-test"} 0`; its textfile scrape error metric remained 0. |
| 01:01:30.815 | Prometheus reported the existing `AppHashMismatch` rule firing for `delivery-test`. No other critical alert was firing before the test. |
| 01:01:40 | Grafana logged that its `celestia-rs-critical` rule forwarded one alert to the local notifier. Later forwarding messages alone are not evidence that an email reached the recipient. |
| 01:03:24 | The synthetic textfile was removed. |
| By 01:05:59 | Prometheus no longer listed the test alert, and Grafana's active-alert API returned no alerts. |

The configured contact point had one recipient and a nonempty SMTP
password file; neither the recipient address nor the password was
read into this record. A recipient/provider delivery acknowledgment
is still required before claiming end-to-end email delivery.

The provisioned Grafana rule uses `execErrState: Alerting`, so a
Prometheus query error is configured to alert through the same
contact point. This path was documented from the deployed rule, not
induced against live monitoring in this test. If the Mac, Grafana or
its network connection is down, Grafana cannot send email at all.
An independent off-host monitor would be needed to cover that failure.

Until the recipient confirms both firing and resolved emails, this
records alert evaluation and local forwarding only. Issue
[#300](https://github.com/celestiaorg/celestia-rs/issues/300) and the
C1.5 delivery gate remain open.
