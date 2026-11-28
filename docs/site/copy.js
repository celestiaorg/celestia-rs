// Home page quickstart: the node options update the commands on the page and
// the agent prompt that "Copy setup prompt" puts on the clipboard. Without
// JavaScript the page shows the default (Mainnet, full node, pruned) commands.

const NETWORKS = {
  mainnet: {
    name: "Mainnet",
    chain: "celestia",
    rpc: "https://rpc.celestia.pops.one:443",
    witness: "https://celestia-rpc.polkachu.com:443",
    funds: "real TIA (there is no faucet); the stake to enter the active set is whatever the 100th bonded validator holds",
  },
  mocha: {
    name: "Mocha testnet",
    chain: "mocha-5",
    rpc: "https://rpc-mocha.pops.one:443",
    witness: "https://celestia-testnet-rpc.polkachu.com:443",
    funds: "testnet TIA from the faucet at https://mocha.celenium.io/faucet (1 TIA to self-delegate plus about 1 TIA for fees)",
  },
};

const RETENTION = {
  pruned: {
    name: "pruned",
    comment: "# Pruned: keep recent state and 3,000 blocks; serve snapshots to peers.",
    settings: [
      "app pruning default",
      "app min-retain-blocks 3000",
      "app state-sync.snapshot-interval 1500",
    ],
    prompt:
      'Keep the default pruning: app.toml pruning = "default", min-retain-blocks = 3000, state-sync.snapshot-interval = 1500.',
  },
  archive: {
    name: "archive",
    comment: "# Archive: keep every block and app state from the snapshot height on.",
    settings: [
      "app pruning nothing",
      "app min-retain-blocks 0",
      "config tx_index.indexer kv",
    ],
    prompt:
      'Run as an archive node: app.toml pruning = "nothing" and min-retain-blocks = 0, and config.toml tx_index.indexer = "kv", set before the first start. History starts at the snapshot height. Warn me if the disk is smaller than 4 TB.',
  },
};

const HOME = "$HOME/.celestia-rs";
const BIN = `${HOME}/bin/celestia-rs`;

function options() {
  const form = document.getElementById("config");
  const value = (name) => form.querySelector(`input[name="${name}"]:checked`).value;
  return { network: value("network"), role: value("role"), retention: value("retention") };
}

function installCommand({ network }) {
  return (
    "curl --proto '=https' --tlsv1.2 -fsSL https://raw.githubusercontent.com/celestiaorg/celestia-rs/main/scripts/install.sh" +
    ` | sh -s -- --channel ${network}\n~/.celestia-rs/bin/celestia-rs version --long`
  );
}

function launchCommand({ network, role, retention }) {
  const n = NETWORKS[network];
  const r = RETENTION[retention];
  const moniker = role === "validator" ? "my-validator" : "my-node";
  return `(
  set -eu
  node="${BIN}"
  home_dir="${HOME}/${network}"
  chain=${n.chain}
  rpc=${n.rpc}
  witness=${n.witness}
  latest=$(curl -fsSL "$rpc/block" | jq -er --arg c "$chain" '.result.block.header | select(.chain_id == $c) | .height | tonumber')
  height=$((latest - 3000))
  hash_at() {
    curl -fsSL "$1/block?height=$height" | jq -er --arg c "$chain" --arg h "$height" '
      .result | select(.block.header.chain_id == $c and .block.header.height == $h)
      | .block_id.hash | select(test("^[0-9A-Fa-f]{64}$"))'
  }
  hash=$(hash_at "$rpc")
  if [ "$hash" != "$(hash_at "$witness")" ]; then
    echo "RPCs disagree at height $height. Stopping." >&2
    exit 1
  fi
  mkdir "$home_dir"  # first launch only: needs a fresh directory
  if ! "$node" init ${moniker} --network ${network} --home "$home_dir" >"$home_dir/init.log" 2>&1; then
    tail -n 10 "$home_dir/init.log" >&2
    exit 1
  fi
  rm "$home_dir/init.log"
  cfg() { "$node" config set "$@" --home "$home_dir"; }
  cfg config statesync.rpc_servers "$rpc,$witness"
  cfg config statesync.trust_height "$height"
  cfg config statesync.trust_hash "$hash"
  cfg config statesync.enable true
  ${r.comment}
${r.settings.map((s) => `  cfg ${s}`).join("\n")}
  ulimit -n 4096
  exec "$node" start --network ${network} --home "$home_dir"
)`;
}

function statusCommand({ network }) {
  return `~/.celestia-rs/bin/celestia-rs status --home ~/.celestia-rs/${network} | jq '.sync_info | {latest_block_height, catching_up}'`;
}

function restartCommand({ network }) {
  return `~/.celestia-rs/bin/celestia-rs start --network ${network} --home ~/.celestia-rs/${network}`;
}

function validatorCommand({ network }) {
  const n = NETWORKS[network];
  return `node=~/.celestia-rs/bin/celestia-rs
home_dir=~/.celestia-rs/${network}
# 1. Operator key: prints the mnemonic once. Write it down offline.
"$node" keys add operator --keyring-backend file --home "$home_dir"
"$node" keys show operator -a --keyring-backend file --home "$home_dir"
# 2. Fund that address. 3. When catching_up is false, register:
cat > "$home_dir/validator.json" <<JSON
{"pubkey": $("$node" comet show-validator --home "$home_dir"),
 "amount": "1000000utia", "moniker": "my-validator",
 "commission-rate": "0.20", "commission-max-rate": "0.30",
 "commission-max-change-rate": "0.01", "min-self-delegation": "1000000"}
JSON
"$node" tx staking create-validator "$home_dir/validator.json" --from operator \\
  --keyring-backend file --home "$home_dir" --chain-id ${n.chain} \\
  --gas auto --gas-adjustment 1.3 --gas-prices 0.004utia --wait`;
}

function buildPrompt(o) {
  const n = NETWORKS[o.network];
  const r = RETENTION[o.retention];
  const validator = o.role === "validator";
  const lines = [
    `Set up and run a celestia-rs ${validator ? "validator" : "non-validating full node"} on Celestia ${n.name} (chain ID ${n.chain}) on this machine, as a ${r.name} node.`,
    "Read https://github.com/celestiaorg/celestia-rs/blob/main/docs/operator-guide.md first.",
    `Check the OS, CPU architecture, memory, free disk, open-file limit, existing node homes, running processes and port conflicts. The requirement is 8 CPU cores, 16 GB RAM and a ${o.retention === "archive" ? "4 TB" : "500 GB"} NVMe SSD; tell me if this machine is below it.`,
    "Leave existing services, keys and firewall rules alone. If the host cannot run the node safely, explain the blocker before changing anything.",
    `Install with the official release installer and --channel ${o.network}, then run celestia-rs version --long.`,
    `Use a fresh user-owned home at ~/.celestia-rs/${o.network} and initialize it with --network ${o.network}.`,
    `Configure P2P state sync with a trust height about 3,000 blocks below the tip and a block hash that matches at ${n.rpc} and ${n.witness}. Stop if they disagree. Do not replay from genesis.`,
    r.prompt,
    "Run ulimit -n 4096, start the node as my user with logs, and keep management endpoints local-only. Leave the DA bridge disabled.",
    "Check the local status twice and report the chain ID, heights, peer count and catching_up. If state sync finds no snapshot, load live peers from a public RPC's /net_info as the operator guide describes.",
  ];
  if (validator) {
    lines.push(
      "When catching_up is false, set up the validator following the operator guide sections 4, 11 and 12.",
      "Create an operator key in the file keyring and have me write the mnemonic down offline; never print it again or store it anywhere else. Back up config/priv_validator_key.json and tell me where.",
      `Show me the operator address and wait for me to fund it with ${n.funds}. Check the balance.`,
      `Prepare validator.json with the consensus public key from comet show-validator, commission-rate 0.20 or more, and the amount, moniker and min-self-delegation I choose. Run create-validator with --generate-only and show me the result. Only after I confirm the amount, moniker and commission in chat, sign and broadcast it with --chain-id ${n.chain} through the local node.`,
      "Then confirm the validator is bonded and that our consensus address signs the latest commits.",
      "Never run a second process with the same signing key.",
    );
    if (o.network === "mainnet") {
      lines.push(
        "Tell me that the project's general Mainnet validator recommendation is still pending (ADR 0008) and suggest a Mocha trial first.",
      );
    }
  } else {
    lines.push("Do not create keys, fund a wallet or submit transactions.");
  }
  lines.push(
    "Finally give me the home and log paths and the exact commands to check progress, stop gracefully and restart.",
  );
  return lines.join(" ");
}

function render() {
  const o = options();
  const commands = {
    install: installCommand(o),
    launch: launchCommand(o),
    status: statusCommand(o),
    restart: restartCommand(o),
    validator: validatorCommand(o),
  };
  for (const el of document.querySelectorAll("[data-cmd]")) {
    el.textContent = commands[el.dataset.cmd];
  }
  for (const el of document.querySelectorAll("[data-show]")) {
    const want = el.dataset.show;
    el.hidden = !(
      (want === "validator" && o.role === "validator") ||
      (want === "mainnet-validator" && o.role === "validator" && o.network === "mainnet")
    );
  }
  const summary = [
    NETWORKS[o.network].name,
    o.role === "validator" ? "validator" : "full node",
    RETENTION[o.retention].name,
  ].join(" · ");
  for (const el of document.querySelectorAll("[data-summary]")) el.textContent = summary;
  document.getElementById("feedback").textContent = "";
  document.getElementById("prompt-fallback").hidden = true;
}

document.getElementById("config").addEventListener("change", render);
render();
document.getElementById("config").hidden = false;
document.querySelector(".agent").hidden = false;

document.getElementById("copy-prompt").addEventListener("click", async () => {
  const prompt = buildPrompt(options());
  const feedback = document.getElementById("feedback");
  const fallback = document.getElementById("prompt-fallback");
  try {
    await navigator.clipboard.writeText(prompt);
    fallback.hidden = true;
    feedback.textContent = "Prompt copied. Paste it into your agent.";
  } catch {
    fallback.value = prompt;
    fallback.hidden = false;
    feedback.textContent = "Copy the selected prompt below.";
    fallback.focus();
    fallback.select();
  }
});

// A copy button on every command block.
for (const block of document.querySelectorAll(".terminal")) {
  const code = block.querySelector("code");
  const b = document.createElement("button");
  b.type = "button";
  b.className = "copy";
  b.textContent = "Copy";
  b.setAttribute("aria-label", "Copy command");
  b.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(code.textContent);
      b.textContent = "Copied";
    } catch {
      b.textContent = "Select to copy";
    }
    setTimeout(() => (b.textContent = "Copy"), 1600);
  });
  block.prepend(b);
}
