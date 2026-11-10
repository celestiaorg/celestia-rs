# Production alert-delivery test — 2026-11-10

Issue [#300](https://github.com/celestiaorg/celestia-rs/issues/300) remains
**open**. The authorized synthetic test exercised the existing production
Prometheus and Grafana rules and reached the configured local notifier.
Firing and resolution at the intended recipient are **not yet established**:
there is no recipient or provider delivery acknowledgment in the retained
evidence. Local forwarding is not mailbox receipt.

All times below are UTC. Test identity:
`delivery-test-20261110T231648Z`.

| Time | Evidence and interpretation |
|---|---|
| 23:16:48 onward | Read deployed rule, policy and contact-point APIs through existing anonymous viewer access; before-test alert lists were empty. No credential file was opened. |
| 23:17:59.146076 | Added one temporary synthetic textfile sample, with a unique `network` label. Its initial missing HELP/TYPE declarations caused node-exporter to omit that synthetic sample; this did not establish alert firing. |
| 23:19:09.440546 | Corrected only the test file to use the existing HELP text and gauge TYPE. Exporter exposed test value 0 alongside real mainnet/Mocha values 1, with textfile scrape error 0. |
| 23:19:15.815289787 | Existing Prometheus `AppHashMismatch` production rule became firing, according to its `activeAt`. First observed firing in the API at 23:19:29.149644. |
| 23:19:40 | Existing Grafana rule became firing for this network. Alertmanager recorded receiver `operator-email`, active state, no inhibition and no silence. Local-notifier forwarding was logged at 23:19:40.031049. |
| 23:20:29.148000 | Removed the temporary metric file; removal check confirmed absent. |
| By 23:20:44.147542 | Prometheus no longer listed the test alert. Its exact resolution time is bounded by the last firing and first absent observations. |
| 23:20:40 | Grafana recorded `Normal (NoData)` for the test instance; observed at 23:20:44.152465. This is rule resolution, not proof of a resolved email. |

The configured notification group wait is 30 seconds and group interval is
five minutes. These settings describe scheduling, not actual send timestamps.
SMTP submission, provider acceptance, firing-email receipt and resolved-email
receipt timestamps remain unknown. The recipient was explicitly asked in this
chat to confirm both messages separately with UTC times and the test identity.
No response is inferred from elapsed time.

## Rule, labels and route

The installed Prometheus rule is `AppHashMismatch` in `celestia-rs-node`:
`celestia_rs_apphash_match == 0`, severity `critical`, no pending duration.
Its synthetic alert labels include `network=delivery-test-20261110T231648Z`,
`job=node`, `instance=127.0.0.1:9100`, and `severity=critical`.

Grafana rule UID `celestia-rs-critical`, title `celestia-rs critical`, folder
`celestia-rs`, group `critical`, evaluates every 30 seconds. Its query selects
firing critical Prometheus alerts, excludes `LowPeers`, and copies the source
alert name into `problem` before aggregation. The observed Grafana alert had
`problem=AppHashMismatch`, the unique test network, `severity=critical`, and
its existing rule/folder labels. The unique network puts the test in a
separate notification group without changing or suppressing real alerts.

The actual policy and contact-point APIs identify default receiver and email
contact-point UID/name `operator-email`, grouped by `network`, 30s group wait,
5m group interval, 4h repeat interval. `disableResolveMessage=false`.
Exactly one existing recipient was configured; its identity is represented by
SHA-256 `90d5f1ebdfd0b8a693c0d0c58e13adbde35c9320cb56a4088edc4d43e3ddc990`
in sanitized evidence. No recipient was changed. Non-secret installed settings
identify enabled SMTP through `smtp.resend.com:587`; SMTP credentials and
secret environment files were not inspected. Grafana used its existing
notification transport configuration.

## What was exercised

**Production evaluation:** a synthetic input metric was scraped by the existing
node-exporter/Prometheus path and evaluated by the unchanged production
`AppHashMismatch` and Grafana critical rules. Their firing and subsequent
normal states were observed. This was not an injected preformed Alertmanager
notification, and no contact-point test API was used.

**Isolated evaluation:** `promtool test rules` separately tested a private copy
of the installed rules, using an isolated synthetic series with value 0 then 1.
Expected firing and resolution passed. That offline test sent no notification
and provides no recipient-delivery evidence.

**Notification evidence:** Grafana's local-notifier forwarding, active routing
and rule resolution are recorded separately. None proves email delivery.
A firing/normal transition is not a firing/resolved message acknowledgment.
The receiver-status API probes returned 404 and provided no delivery history.

## Monitoring failures and operational limits

The deployed rule API reports `execErrState=Alerting`, `noDataState=OK` and
`isPaused=false`. A failed Prometheus query is configured to enter alerting
and use the default `operator-email` route; the fallback annotation describes
Grafana being unable to query Prometheus. A successful query returning no
critical alerts instead follows `noDataState=OK`, as observed during this
resolution. These are distinct cases.

The query-error behavior is **documented from deployed configuration**, not
induced against live monitoring. Its recipient delivery is not established by
this test. Neither Prometheus nor Grafana was stopped or disconnected to test
failure. When Grafana or the host is down, this on-host route cannot send
notifications; an independent off-host monitor is needed to cover that case.

No validator was restarted, deployed, stopped or reconfigured for this test.
No signing key, keyring, SMTP credential or secret environment file was read.
The initial test-format error, subsequent correction and successful exporter
exposure are retained; the initial injection time must not be substituted for
the later production firing time.

Cleanup was confirmed at **23:25:30.080660Z**, after the configured grouping
interval: both the synthetic `.prom` file and its staging file were absent,
and Prometheus/Grafana active-alert lists were empty. The production rule,
notification policy and contact-point API bodies match their before-test
snapshots; installed alerting, Prometheus and rule files are byte-identical.
Validator PIDs 24144/47956 and monitoring PIDs 19319/19325/19331, including
their start times, were unchanged across collection. Retained ten-minute
histories spanning the test show real mainnet/Mocha app-hash values 1 and
all four monitoring scrape targets up at every available sample. No test
service, silence, notification-policy change or contact-point change was made.
Historical test samples and inactive alert state remain as monitoring history.

## Evidence and remaining work

Raw request/response observations, non-secret installed configuration snapshots,
test mechanism, isolated evaluation output and selected notification logs are
retained in durable owner-only storage at
`~/.local/share/celestia-rs-audits/issue-300/20261110T231648Z/`.
They are not stored solely in temporary storage and are not published.

[Sanitized production observations](evidence/alert-delivery-2026-11-10/production-observations.json)
retain UTC state samples, labels and receiver identity while omitting private
Grafana URLs and the recipient address. The
[summary](evidence/alert-delivery-2026-11-10/summary.json) distinguishes production
and isolated tests, and records configured routing, cleanup and limitations.
The [private-source manifest](evidence/alert-delivery-2026-11-10/private-source-manifest.json)
retains source filenames, sizes and SHA-256 digests; [digests](evidence/alert-delivery-2026-11-10/digests.json)
identify the sanitized outputs. Request timestamps are observation times,
not email-delivery timestamps.

Remaining blockers are explicit recipient/provider acknowledgments for **both**
the firing and resolved messages, tied to this test identity, with UTC
acknowledgment/receipt timestamps. “Sent” or “forwarded” logs cannot close that
gap. Until those are supplied and retained, this is evidence progress and
contains no closing reference for #300.
