#!/usr/bin/env bash
# Register a short-lived key only on the existing benchmark instance.
set -euo pipefail
project=fibreda
zone=us-central1-a
vm=celestia-rs-genesis-sync
cd "$RUNNER_TEMP"
umask 077
ssh-keygen -q -t ed25519 -N '' -f bench-key
instance=$(gcloud compute instances describe "$vm" --project="$project" --zone="$zone" --format=json)
jq -e '.labels.campaign == "celestia-rs-genesis-sync" and .status == "RUNNING"' <<<"$instance" >/dev/null
ip=$(jq -er '.networkInterfaces[0].accessConfigs[0].natIP' <<<"$instance")
host_key=$(gcloud compute instances get-guest-attributes "$vm" --project="$project" --zone="$zone" \
  --query-path=hostkeys/ssh-ed25519 --format='value(value)')
[[ "$host_key" =~ ^[A-Za-z0-9+/]{68}$ ]]
printf '%s ssh-ed25519 %s\n' "$ip" "$host_key" > known_hosts
expiry=$(date -u -d '+6 hours' +%Y-%m-%dT%H:%M:%S+0000)
read -r kind key _ < bench-key.pub
entry="bench:$kind $key google-ssh {\"userName\":\"bench\",\"expireOn\":\"$expiry\"}"
# Send the original fingerprint so a concurrent metadata edit yields a conflict.
jq --arg entry "$entry" '.metadata | {fingerprint,items} |
  .items |= (map(select(.key != "ssh-keys")) + [
    {key:"ssh-keys",value:([.[] | select(.key == "ssh-keys") | .value] + [$entry] | join("\n"))}])' \
  <<<"$instance" > metadata.json
curl --fail --silent --show-error -H "Authorization: Bearer $CLOUDSDK_AUTH_ACCESS_TOKEN" \
  --json @metadata.json "https://compute.googleapis.com/compute/v1/projects/$project/zones/$zone/instances/$vm/setMetadata" >/dev/null
cat > bench-config <<CONFIG
Host campaign
  HostName $ip
  User bench
  IdentityFile $RUNNER_TEMP/bench-key
  IdentitiesOnly yes
  BatchMode yes
  ConnectTimeout 8
  StrictHostKeyChecking yes
  UserKnownHostsFile $RUNNER_TEMP/known_hosts
CONFIG
for attempt in {1..12}; do
  if ssh -F bench-config campaign true; then exit 0; fi
  sleep 5
done
exit 1
