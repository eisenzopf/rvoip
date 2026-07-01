#!/usr/bin/env bash
# Validate and publish the rvoip 0.2.2 beta release train in dependency order.
#
# Default mode validates and runs cargo publish --dry-run only. Use --execute
# to validate, dry-run each package, then publish for real.

set -euo pipefail

expected_version="0.2.2"
mode="dry-run"
allow_dirty=0
wait_seconds=60
skip_validate=0
index_poll_seconds=15
index_timeout_seconds=900

tier_0=(
  rvoip-core-traits
  rvoip-infra-common
  rvoip-sip-core
  rvoip-codec-core
)
tier_1=(
  rvoip-auth-core
  rvoip-rtp-core
  rvoip-sip-transport
)
tier_2=(
  rvoip-media-core
  rvoip-sip-dialog
)
tier_3=(
  rvoip-sip-proxy
  rvoip-sip-registrar
)
tier_4=(rvoip-core)
tier_5=(rvoip-sip)
tier_6=(rvoip)
beta_packages=(
  "${tier_0[@]}"
  "${tier_1[@]}"
  "${tier_2[@]}"
  "${tier_3[@]}"
  "${tier_4[@]}"
  "${tier_5[@]}"
  "${tier_6[@]}"
)

usage() {
  cat <<'EOF'
Usage: scripts/publish_beta_train_0.2.2.sh [--dry-run|--execute] [options]

Modes:
  --dry-run       Validate, then cargo publish --dry-run Tier 0. Default.
  --execute       Validate, dry-run each package, then publish it for real.

Options:
  --allow-dirty   Allow publishing from a dirty working tree.
  --skip-validate Skip cargo/test validation and only run publish dry-runs/publish.
  --wait SECONDS  Initial wait before crates.io visibility polling. Default: 60.
  -h, --help      Show this help.

This script publishes only the beta train:
  rvoip, rvoip-core, rvoip-core-traits, rvoip-infra-common, rvoip-auth-core,
  rvoip-codec-core, rvoip-media-core, rvoip-rtp-core, rvoip-sip,
  rvoip-sip-core, rvoip-sip-transport, rvoip-sip-dialog, rvoip-sip-proxy,
  and rvoip-sip-registrar.

Validation is intentionally front-loaded so one --execute run proves the train
still builds before any crate is published. The script refreshes the workspace
lockfile with cargo update -w before locked validation. In --execute mode it
polls crates.io after each published tier until that tier's packages are
visible before attempting dependent packages.

Before Tier 0 exists on crates.io, Cargo cannot dry-run later tiers because
their 0.2.2 internal dependencies are not resolvable from the index yet.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run)
      mode="dry-run"
      shift
      ;;
    --execute)
      mode="execute"
      shift
      ;;
    --allow-dirty)
      allow_dirty=1
      shift
      ;;
    --skip-validate)
      skip_validate=1
      shift
      ;;
    --wait)
      if [[ $# -lt 2 || ! "$2" =~ ^[0-9]+$ ]]; then
        echo "error: --wait requires a nonnegative integer number of seconds" >&2
        exit 2
      fi
      wait_seconds="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
done

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
cd "$repo_root"

log_dir="$repo_root/target/release-logs"
mkdir -p "$log_dir"
log_file="$log_dir/publish_beta_train_${expected_version}_$(date -u +%Y%m%dT%H%M%SZ).log"
exec > >(tee -a "$log_file") 2>&1
trap 'status=$?; if [[ "$status" -ne 0 ]]; then echo; echo "Publish script failed with status $status."; echo "Log: $log_file"; fi' EXIT

workspace_version="$(
  perl -ne '
    if (/^\[workspace\.package\]\s*$/) { $in = 1; next }
    if ($in && /^\[/) { exit }
    if ($in && /^\s*version\s*=\s*"([^"]+)"/) { print $1; exit }
  ' Cargo.toml
)"

if [[ "$workspace_version" != "$expected_version" ]]; then
  echo "error: expected workspace version $expected_version, found ${workspace_version:-<missing>}" >&2
  exit 1
fi

if [[ "$allow_dirty" -ne 1 && -n "$(git status --porcelain)" ]]; then
  echo "error: working tree is dirty; commit or stash changes before publishing" >&2
  echo "       use --allow-dirty only if you intentionally want to publish this tree" >&2
  git status --short >&2
  exit 1
fi

cargo_publish_args=(--locked)
if [[ "$allow_dirty" -eq 1 ]]; then
  cargo_publish_args+=(--allow-dirty)
fi

run_step() {
  local name="$1"
  shift

  echo
  echo "==> validate: $name"
  "$@"
}

expect_no_matches() {
  local name="$1"
  shift

  echo
  echo "==> validate: $name"
  local output status
  set +e
  output="$("$@" 2>&1)"
  status=$?
  set -e
  if [[ "$status" -eq 0 ]]; then
    echo "error: unexpected matches found for $name" >&2
    echo "$output" >&2
    exit 1
  fi
  if [[ "$status" -ne 1 ]]; then
    echo "$output" >&2
    exit "$status"
  fi
}

expect_no_matches_in_files() {
  local name="$1"
  local pattern="$2"
  shift 2

  echo
  echo "==> validate: $name"

  if [[ "$#" -eq 0 ]]; then
    return 0
  fi

  local output status
  set +e
  output="$(grep -n -E "$pattern" "$@" 2>&1)"
  status=$?
  set -e
  if [[ "$status" -eq 0 ]]; then
    echo "error: unexpected matches found for $name" >&2
    echo "$output" >&2
    exit 1
  fi
  if [[ "$status" -ne 1 ]]; then
    echo "$output" >&2
    exit "$status"
  fi
}

verify_beta_lock_versions() {
  echo
  echo "==> validate: beta train lockfile versions"

  local package version failed
  failed=0
  for package in "${beta_packages[@]}"; do
    version="$(
      LOCK_PACKAGE="$package" perl -0ne '
        my $name = quotemeta($ENV{"LOCK_PACKAGE"});
        if (/\[\[package\]\]\nname = "$name"\nversion = "([^"]+)"/) {
          print $1;
          exit;
        }
      ' Cargo.lock
    )"

    if [[ "$version" != "$expected_version" ]]; then
      echo "error: Cargo.lock has ${package} ${version:-<missing>}; expected $expected_version" >&2
      failed=1
    fi
  done

  if [[ "$failed" -ne 0 ]]; then
    exit 1
  fi
}

refresh_workspace_lockfile() {
  run_step "workspace lockfile refresh" \
    cargo update -w

  verify_beta_lock_versions
}

validate_release_train() {
  run_step "beta train manifest sync" \
    scripts/sync_beta_train_versions.sh --check

  refresh_workspace_lockfile

  run_step "locked workspace metadata" \
    bash -c 'cargo metadata --locked --no-deps --format-version 1 > /tmp/rvoip-0.2.2-metadata.json'

  run_step "workspace all-target check" \
    cargo check --workspace --locked --all-targets

  run_step "codec-core G.729 tests" \
    cargo test -p rvoip-codec-core --locked --features g729 g729 -- --nocapture

  run_step "media-core G.729 tests" \
    cargo test -p rvoip-media-core --locked --features g729 g729

  run_step "rvoip-sip library tests" \
    cargo test -p rvoip-sip --locked --lib

  run_step "rvoip-sip integration tests" \
    cargo test -p rvoip-sip --locked --tests --features generated-validation,dev-insecure-tls

  run_step "sip-core RFC 4475 torture tests" \
    cargo test -p rvoip-sip-core --locked --features lenient_parsing --test torture_tests

  run_step "sip-core generated message compliance" \
    cargo test -p rvoip-sip-core --locked --features generated-validation --test generated_message_compliance

  run_step "sip-dialog generated SIP compliance" \
    cargo test -p rvoip-sip-dialog --locked --features generated-validation --test generated_sip_compliance

  expect_no_matches_in_files "stale 0.2.1/0.2.0 example manifests" \
    'version = "0\.2\.[01]"' \
    $(find examples -type f -name Cargo.toml -print)

  expect_no_matches_in_files "stale 0.2.1/0.2.0 release docs" \
    '0\.2\.[01]' \
    README.md \
    crates/rvoip/README.md \
    $(find crates -mindepth 3 -maxdepth 3 -type f -name README.md -print) \
    $(find crates/sip/rvoip-sip/docs -maxdepth 1 -type f -name 'BETA_*.md' -print)
}

publish_pkg() {
  local package="$1"

  if crate_version_visible "$package"; then
    echo
    echo "==> skip: ${package}@${expected_version} is already visible on crates.io"
    return 0
  fi

  echo
  echo "==> dry-run: $package"
  cargo publish -p "$package" "${cargo_publish_args[@]}" --dry-run

  if [[ "$mode" == "execute" ]]; then
    echo
    echo "==> publish: $package"
    cargo publish -p "$package" "${cargo_publish_args[@]}"
    published_this_tier=1
  fi
}

publish_tier() {
  local tier="$1"
  shift
  published_this_tier=0

  echo
  echo "================================================================"
  echo "Tier $tier"
  echo "================================================================"

  local package
  for package in "$@"; do
    publish_pkg "$package"
  done

  if [[ "$mode" == "execute" && "$published_this_tier" -eq 1 && "$wait_seconds" -gt 0 ]]; then
    echo
    echo "Waiting initial ${wait_seconds}s for crates.io index propagation..."
    sleep "$wait_seconds"
  fi

  if [[ "$mode" == "execute" ]]; then
    wait_for_tier_visible "$tier" "$@"
  fi
}

crate_version_visible() {
  local package="$1"
  cargo info --registry crates-io "${package}@${expected_version}" >/dev/null 2>&1
}

all_crate_versions_visible() {
  local package
  for package in "$@"; do
    if ! crate_version_visible "$package"; then
      return 1
    fi
  done
  return 0
}

wait_for_tier_visible() {
  local tier="$1"
  shift

  local deadline=$((SECONDS + index_timeout_seconds))
  local pending=("$@")
  local next_pending package

  echo
  echo "Checking crates.io visibility for Tier $tier..."

  while ((${#pending[@]} > 0)); do
    next_pending=()
    for package in "${pending[@]}"; do
      if crate_version_visible "$package"; then
        echo "  visible: ${package}@${expected_version}"
      else
        next_pending+=("$package")
      fi
    done

    if ((${#next_pending[@]} == 0)); then
      return 0
    fi

    if ((SECONDS >= deadline)); then
      echo "error: crates.io did not show Tier $tier packages before timeout:" >&2
      for package in "${next_pending[@]}"; do
        echo "  ${package}@${expected_version}" >&2
      done
      exit 1
    fi

    echo "  waiting for: ${next_pending[*]}"
    sleep "$index_poll_seconds"
    pending=("${next_pending[@]}")
  done
}

echo "Publishing beta train $expected_version from $repo_root"
echo "Mode: $mode"
echo "Validation: $([[ "$skip_validate" -eq 1 ]] && echo skipped || echo enabled)"
echo "Log: $log_file"

if [[ "$skip_validate" -ne 1 ]]; then
  validate_release_train
else
  refresh_workspace_lockfile
fi

publish_tier 0 "${tier_0[@]}"

if [[ "$mode" == "dry-run" ]] && ! all_crate_versions_visible "${tier_0[@]}"; then
  echo
  echo "Beta train $expected_version Tier 0 dry-run completed."
  echo "Later tiers require Tier 0 $expected_version packages to exist on crates.io."
  echo "Use --execute to publish Tier 0, wait for index visibility, and continue."
  exit 0
fi

publish_tier 1 "${tier_1[@]}"
publish_tier 2 "${tier_2[@]}"
publish_tier 3 "${tier_3[@]}"
publish_tier 4 "${tier_4[@]}"
publish_tier 5 "${tier_5[@]}"

if [[ "$mode" == "dry-run" ]] && ! all_crate_versions_visible "${tier_5[@]}"; then
  echo
  echo "Beta train $expected_version dry-run reached Tier 5."
  echo "The facade crate requires rvoip-sip $expected_version to exist on crates.io."
  echo "Use --execute to publish rvoip-sip, wait for index visibility, and publish rvoip."
  exit 0
fi

publish_tier 6 "${tier_6[@]}"

echo
if [[ "$mode" == "execute" ]]; then
  echo "Beta train $expected_version publish commands completed."
else
  echo "Beta train $expected_version dry-run completed. Re-run with --execute to publish."
fi
