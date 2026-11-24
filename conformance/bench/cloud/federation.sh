#!/usr/bin/env bash
# Run in Google Cloud Shell. The default mode only lists GitHub OIDC providers.
set -euo pipefail
project=${1:?usage: bash federation.sh PROJECT_ID [check|setup]}
mode=${2:-check}
case "$mode" in check|setup) ;; *) echo 'expected check or setup' >&2; exit 2;; esac
pool=celestia-rs-bench
provider=github
account=celestia-rs-bench
email="$account@$project.iam.gserviceaccount.com"
issuer=https://token.actions.githubusercontent.com
number=$(gcloud projects describe "$project" --format='value(projectNumber)')

# gcloud flags take the project ID. IAM resource names use its number.
location=(--project="$project" --location=global)

# IAM can acknowledge creation before a dependent API sees the new resource.
retry_not_found() {
  local delay output errors status
  errors=$(mktemp)
  for delay in 2 4 8 16 30 60 0; do
    if output=$("$@" 2>"$errors"); then
      cat "$errors" >&2
      rm -f "$errors"
      printf '%s\n' "$output"
      return 0
    else
      status=$?
    fi
    if [[ $delay == 0 ]] || ! grep -q 'NOT_FOUND' "$errors"; then
      cat "$errors" >&2
      rm -f "$errors"
      return "$status"
    fi
    echo "IAM resource not visible yet; retrying in ${delay}s." >&2
    sleep "$delay"
  done
}

if [[ $mode == check ]]; then
  pools=$(gcloud iam workload-identity-pools list "${location[@]}" --format='value(name)')
  found=0
  for path in $pools; do
    matches=$(retry_not_found gcloud iam workload-identity-pools providers list "${location[@]}" \
      --workload-identity-pool="${path##*/}" --filter="oidc.issuerUri=$issuer" \
      --format='value(name)')
    if [[ -n $matches ]]; then
      echo "$matches"
      found=1
    fi
  done
  if [[ $found == 0 ]]; then echo "No GitHub OIDC providers found in $project."; fi
  exit 0
fi

# Numeric claims bind trust to the immutable repository and organization IDs.
repo_id=1239115574
owner_id=54859940
condition="assertion.repository_id=='$repo_id' && assertion.repository_owner_id=='$owner_id' && assertion.ref=='refs/heads/main' && assertion.event_name=='workflow_dispatch' && assertion.workflow_ref=='celestiaorg/celestia-rs/.github/workflows/operator-benchmark.yml@refs/heads/main'"
mapping='google.subject=assertion.sub,attribute.repository_id=assertion.repository_id'
gcloud services enable iam.googleapis.com iamcredentials.googleapis.com sts.googleapis.com --project="$project"
pools=$(gcloud iam workload-identity-pools list "${location[@]}" --format='value(name)')
if ! grep -Fxq "projects/$number/locations/global/workloadIdentityPools/$pool" <<<"$pools"; then
  gcloud iam workload-identity-pools create "$pool" "${location[@]}" --display-name="Celestia operator benchmarks"
fi
base="projects/$number/locations/global/workloadIdentityPools/$pool"
full="$base/providers/$provider"
providers=$(retry_not_found gcloud iam workload-identity-pools providers list "${location[@]}" --workload-identity-pool="$pool" --format='value(name)')
if [[ -z $providers ]]; then
  retry_not_found gcloud iam workload-identity-pools providers create-oidc "$provider" "${location[@]}" \
    --workload-identity-pool="$pool" --display-name='GitHub benchmark workflow' \
    --issuer-uri="$issuer" --attribute-mapping="$mapping" --attribute-condition="$condition"
elif [[ $providers == "$full" ]]; then
  gcloud iam workload-identity-pools providers describe "$provider" "${location[@]}" \
    --workload-identity-pool="$pool" --format=json | jq -e --arg issuer "$issuer" --arg cond "$condition" \
    '.oidc.issuerUri == $issuer and .attributeCondition == $cond and
     .attributeMapping == {"google.subject":"assertion.sub","attribute.repository_id":"assertion.repository_id"} and
     (.disabled != true) and (.oidc.allowedAudiences // [] | length == 0)' >/dev/null || {
       echo 'Existing provider differs from the benchmark configuration. Inspect it before proceeding.' >&2; exit 1; }
else
  echo "Unexpected providers in dedicated pool $pool. Existing IAM configuration was preserved." >&2; exit 1
fi
accounts=$(gcloud iam service-accounts list --project="$project" --format='value(email)')
if ! grep -Fxq "$email" <<<"$accounts"; then
  gcloud iam service-accounts create "$account" --project="$project" --display-name='Celestia operator benchmarks'
elif [[ $(gcloud iam service-accounts describe "$email" --project="$project" --format='value(displayName)') != 'Celestia operator benchmarks' ]]; then
  echo "Existing service account $email has a different purpose. Inspect it before proceeding." >&2; exit 1
fi
retry_not_found gcloud iam service-accounts add-iam-policy-binding "$email" --project="$project" \
  --role=roles/iam.workloadIdentityUser \
  --member="principalSet://iam.googleapis.com/$base/attribute.repository_id/$repo_id" >/dev/null
printf 'Provider: %s\nService account: %s\n' "$full" "$email"
