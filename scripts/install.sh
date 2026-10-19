#!/bin/sh
# Installs or updates celestia-rs from its GitHub release archives.
#
#   curl --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/celestiaorg/celestia-rs/main/scripts/install.sh | sh
#
# Re-run the same command to update. Pass options after `sh -s --`:
#
#   ... | sh -s -- --version v0.1.0     # a specific release
#   ... | sh -s -- --prerelease         # newest release, pre-releases included
#   ... | sh -s -- --no-modify-path     # leave shell profiles alone
#   ... | sh -s -- --uninstall          # remove the binary and the PATH lines
#
# The binary goes to $CELESTIA_RS_HOME/bin (default ~/.celestia-rs/bin, the
# path docs/operator-guide.md uses). ~/.celestia-rs is also the node's
# default --home, so the installer only ever writes bin/celestia-rs and its
# own env files there, and never removes the directory.
#
# Every archive is checked against the release's SHA-256 checksum before it
# is unpacked. The binary is swapped in with a rename, so a node already
# running keeps its old binary until it restarts.
#
# Environment:
#   CELESTIA_RS_HOME     install root (default ~/.celestia-rs)
#   CELESTIA_RS_VERSION  same as --version
#   GITHUB_TOKEN         a GitHub token (or GH_TOKEN): needed while the repo is
#                        private, and lifts the API rate limit. Without one,
#                        a logged-in `gh` CLI's token is used if needed.
#
# POSIX sh: runs under dash, bash, zsh, ksh and busybox ash.

set -eu

REPO=${CELESTIA_RS_REPO:-celestiaorg/celestia-rs}
# Overridable for mirrors and for scripts/test-install.sh.
GITHUB_URL=${CELESTIA_RS_GITHUB_URL:-https://github.com}
GITHUB_API=${CELESTIA_RS_GITHUB_API:-https://api.github.com}
# HTTPS only, unless an override above points at plain http (the tests).
case "$GITHUB_URL $GITHUB_API" in
  *http://*) proto='=https,http' ;;
  *) proto='=https' ;;
esac
# The release builds run on ubuntu-22.04 (.github/workflows/release.yml).
MIN_GLIBC=2.35

say() { printf 'celestia-rs-install: %s\n' "$*" >&2; }
warn() { printf 'celestia-rs-install: warning: %s\n' "$*" >&2; }
err() {
  printf 'celestia-rs-install: error: %s\n' "$*" >&2
  exit 1
}
has() { command -v "$1" >/dev/null 2>&1; }

usage() {
  cat <<EOF
celestia-rs installer

Installs the latest celestia-rs release, or updates an existing install.

USAGE:
    install.sh [OPTIONS]

OPTIONS:
    -v, --version <TAG>   Install this release (e.g. v0.1.0) instead of the latest
        --prerelease      Allow pre-releases when picking the latest release
        --no-modify-path  Don't add the install directory to PATH in shell profiles
        --uninstall       Remove the installed binary and the PATH setup
    -h, --help            Print this help

The binary is installed to \$CELESTIA_RS_HOME/bin (default ~/.celestia-rs/bin).
EOF
}

main() {
  version=${CELESTIA_RS_VERSION:-}
  token=${GITHUB_TOKEN:-${GH_TOKEN:-}}
  tried_gh=0
  libc_note=""
  prerelease=0
  modify_path=1
  uninstall=0
  while [ $# -gt 0 ]; do
    case $1 in
      -v | --version)
        [ $# -ge 2 ] || err "$1 needs a value, e.g. $1 v0.1.0"
        version=$2
        shift
        ;;
      --version=*) version=${1#--version=} ;;
      --prerelease) prerelease=1 ;;
      --no-modify-path) modify_path=0 ;;
      --uninstall) uninstall=1 ;;
      -h | --help)
        usage
        exit 0
        ;;
      *) err "unknown option '$1' (see --help)" ;;
    esac
    shift
  done

  [ -n "${HOME:-}" ] || err "\$HOME is not set"
  home=${CELESTIA_RS_HOME:-$HOME/.celestia-rs}
  bin_dir=$home/bin

  if [ "$uninstall" = 1 ]; then
    do_uninstall
    return
  fi

  need uname
  need tar
  need awk
  need mkdir
  need mv
  need rm
  pick_downloader
  pick_sha256

  target=$(detect_target)
  if [ "$(uname -s)" = Linux ]; then
    check_glibc
  fi

  if [ -z "$version" ]; then
    latest_version "$prerelease"
  fi
  case $version in
    v*) ;;
    *) version=v$version ;;
  esac

  current=$(installed_tag)
  if [ "$current" = "$version" ]; then
    say "celestia-rs $version is already installed at $bin_dir/celestia-rs"
  else
    install_release "$version" "$target"
    if [ -n "$current" ]; then
      say "updated celestia-rs $current -> $version (previous binary: $bin_dir/celestia-rs.prev)"
    elif [ "$replaced" = 1 ]; then
      say "installed celestia-rs $version to $bin_dir/celestia-rs (the binary it replaced is $bin_dir/celestia-rs.prev)"
    else
      say "installed celestia-rs $version to $bin_dir/celestia-rs"
    fi
    note_running_nodes
  fi

  write_env_files
  if [ "$modify_path" = 1 ]; then
    modify_profiles
  fi
  finish_message
}

need() {
  has "$1" || err "need '$1' (command not found)"
}

pick_downloader() {
  if has curl; then
    downloader=curl
  elif has wget; then
    downloader=wget
  else
    err "need 'curl' or 'wget' to download celestia-rs"
  fi
}

pick_sha256() {
  if has sha256sum; then
    sha256=sha256sum
  elif has shasum; then
    sha256=shasum
  elif has openssl; then
    sha256=openssl
  else
    err "need 'sha256sum', 'shasum' or 'openssl' to verify the download"
  fi
}

file_sha256() {
  case $sha256 in
    sha256sum) sha256sum "$1" ;;
    shasum) shasum -a 256 "$1" ;;
    openssl) openssl dgst -sha256 -r "$1" ;;
  esac | cut -d' ' -f1
}

# get URL OUT [HEADER...]: download URL to the file OUT ("-": stdout).
get() {
  url=$1
  out=$2
  shift 2
  n=$#
  while [ "$n" -gt 0 ]; do
    if [ "$downloader" = curl ]; then
      set -- "$@" -H "$1"
    else
      set -- "$@" "--header=$1"
    fi
    shift
    n=$((n - 1))
  done
  if [ "$downloader" = curl ]; then
    curl --proto "$proto" --tlsv1.2 -sSfL --retry 3 "$@" -o "$out" "$url"
  else
    wget -q "$@" -O "$out" "$url"
  fi
}

# api PATH [OUT [ACCEPT]]: a GitHub API request, with $token if there is one.
api() {
  if [ -n "$token" ]; then
    get "$GITHUB_API$1" "${2:--}" "Accept: ${3:-application/vnd.github+json}" "Authorization: Bearer $token"
  else
    get "$GITHUB_API$1" "${2:--}" "Accept: ${3:-application/vnd.github+json}"
  fi
}

# Without a token, a private repo looks like a missing one. A logged-in
# GitHub CLI has a token: use it, once.
use_gh_token() {
  [ -z "$token" ] && [ "$tried_gh" = 0 ] || return 1
  tried_gh=1
  has gh || return 1
  token=$(gh auth token 2>/dev/null) || token=""
  [ -n "$token" ] || return 1
  say "using the GitHub CLI's login (gh auth token)"
}

# Splits JSON so every key is on its own line, whatever its formatting.
# shellcheck disable=SC2020 # one newline per character, for portability
json_lines() { tr ',{}[]' '\n\n\n\n\n'; }

# Prints the first value of a string KEY in JSON on stdin.
json_str() {
  json_lines | sed -n "s/^[[:space:]]*\"$1\":[[:space:]]*\"\([^\"]*\)\".*/\1/p" | head -n 1
}

# Prints the target triple of the release archive for this machine.
detect_target() {
  os=$(uname -s)
  arch=$(uname -m)
  case $os in
    Darwin)
      # A shell under Rosetta reports x86_64 on Apple silicon.
      if [ "$arch" = x86_64 ] && [ "$(sysctl -n hw.optional.arm64 2>/dev/null || true)" = 1 ]; then
        arch=arm64
      fi
      case $arch in
        arm64 | aarch64) echo aarch64-apple-darwin ;;
        *) unsupported "macOS on $arch (release builds are for Apple silicon)" ;;
      esac
      ;;
    Linux)
      case $arch in
        x86_64 | amd64) echo x86_64-unknown-linux-gnu ;;
        aarch64 | arm64) echo aarch64-unknown-linux-gnu ;;
        *) unsupported "Linux on $arch" ;;
      esac
      ;;
    *) unsupported "$os" ;;
  esac
}

unsupported() {
  err "no celestia-rs release build for $1.
Release builds: aarch64-apple-darwin, x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu.
Build from source instead: https://github.com/$REPO#getting-started"
}

# The Linux builds link glibc dynamically, so musl systems (Alpine) can't
# run them. A glibc older than the build host's may or may not, depending on
# the symbols the binary uses: install_release runs the binary before
# installing it, and $libc_note explains a failure.
check_glibc() {
  libc_note=""
  libc=$(getconf GNU_LIBC_VERSION 2>/dev/null || true)
  case $libc in
    glibc\ *)
      have=${libc#glibc }
      if ! version_ge "$have" "$MIN_GLIBC"; then
        libc_note="This system has glibc $have; release builds are made against glibc $MIN_GLIBC."
      fi
      ;;
    *)
      if ls /lib/ld-musl-* >/dev/null 2>&1; then
        unsupported "musl libc (e.g. Alpine; release builds link glibc)"
      fi
      ;;
  esac
}

# version_ge A B: A >= B for dotted numeric versions.
version_ge() {
  a=$1
  b=$2
  while [ -n "$a" ] || [ -n "$b" ]; do
    x=${a%%.*}
    y=${b%%.*}
    [ "$a" = "${a#*.}" ] && a="" || a=${a#*.}
    [ "$b" = "${b#*.}" ] && b="" || b=${b#*.}
    x=${x:-0}
    y=${y:-0}
    [ "$x" -gt "$y" ] && return 0
    [ "$x" -lt "$y" ] && return 1
  done
  return 0
}

# Sets $version to the tag of the newest release: the latest stable one
# unless pre-releases are allowed. With no stable release yet, falls back to
# the newest pre-release. Not run in a subshell: use_gh_token sets $token.
latest_version() {
  if [ "$1" = 0 ]; then
    version=$(latest_stable_tag || true)
    [ -z "$version" ] || return 0
  fi
  # Drafts aren't listed; pre-releases are, newest first.
  if ! json=$(api "/repos/$REPO/releases?per_page=10"); then
    if use_gh_token; then
      latest_version "$1"
      return
    fi
    err "could not list the releases of $REPO.
If the repository is private, set GITHUB_TOKEN (or log in with 'gh auth login').
If GitHub rate-limited this IP, set GITHUB_TOKEN, or pass --version."
  fi
  version=$(printf '%s\n' "$json" | json_str tag_name)
  [ -n "$version" ] || err "$REPO has no releases yet; build from source: https://github.com/$REPO#getting-started"
  if [ "$1" = 0 ]; then
    say "no stable release yet; using the newest pre-release, $version"
  fi
}

# Prints the latest stable release's tag. Without a token, from where
# github.com/<repo>/releases/latest redirects, which isn't rate limited
# like the API.
latest_stable_tag() {
  if [ -n "$token" ]; then
    api "/repos/$REPO/releases/latest" 2>/dev/null | json_str tag_name
    return
  fi
  url="$GITHUB_URL/$REPO/releases/latest"
  if [ "$downloader" = curl ]; then
    final=$(curl --proto "$proto" --tlsv1.2 -sSfL -o /dev/null -w '%{url_effective}' "$url" 2>/dev/null) || return 1
  else
    final=$(wget -S --spider "$url" 2>&1 | sed -n 's/^ *[Ll]ocation: *\([^ ]*\).*/\1/p' | tail -n 1 | tr -d '\r')
  fi
  case $final in
    */releases/tag/?*) echo "${final##*/releases/tag/}" ;;
    *) return 1 ;;
  esac
}

# fetch_assets TAG DIR FILE...: download release assets into DIR: from the
# public download URLs, or else (a private repository) through the API.
fetch_assets() {
  tag=$1
  dir=$2
  shift 2
  ok=1
  for f in "$@"; do
    get "$GITHUB_URL/$REPO/releases/download/$tag/$f" "$dir/$f" 2>/dev/null || ok=0
  done
  [ "$ok" = 0 ] || return 0
  [ -n "$token" ] || use_gh_token || return 1
  release=$(api "/repos/$REPO/releases/tags/$tag") || return 1
  for f in "$@"; do
    # An asset's "url" (the API one) comes before its "name".
    asset=$(printf '%s\n' "$release" | json_lines | awk -v want="$f" '
      /"url":[[:space:]]*"[^"]*\/releases\/assets\/[0-9]+"/ { u = $0; sub(/^[^:]*:[[:space:]]*"/, "", u); sub(/".*/, "", u) }
      /"name":[[:space:]]*"/ { n = $0; sub(/^[^:]*:[[:space:]]*"/, "", n); sub(/".*/, "", n); if (n == want && u != "") { print u; exit } }')
    [ -n "$asset" ] || return 1
    # The API redirects to a storage host that rejects the token. curl drops
    # it on a redirect to another host; wget doesn't, so follow it by hand.
    if [ "$downloader" = curl ]; then
      get "$asset" "$dir/$f" "Accept: application/octet-stream" "Authorization: Bearer $token" || return 1
    else
      loc=$(wget -S --max-redirect=0 -O /dev/null --header="Accept: application/octet-stream" \
        --header="Authorization: Bearer $token" "$asset" 2>&1 |
        sed -n 's/^ *[Ll]ocation: *\([^ ]*\).*/\1/p' | tail -n 1 | tr -d '\r')
      [ -n "$loc" ] && get "$loc" "$dir/$f" || return 1
    fi
  done
}

# install_release TAG TARGET
install_release() {
  name=celestia-rs-$1-$2
  tmp=$(mktemp -d 2>/dev/null || mktemp -d -t celestia-rs)
  # Removed on exit, however the script ends.
  trap 'rm -rf "$tmp"' EXIT
  trap 'exit 1' HUP INT TERM

  say "downloading $name.tar.gz"
  fetch_assets "$1" "$tmp" "$name.tar.gz" "$name.tar.gz.sha256" ||
    err "could not download $name.tar.gz and its .sha256 from release $1 of $REPO.
Is $1 a release with a $2 build? If the repository is private, set GITHUB_TOKEN
(or log in with 'gh auth login')."

  want=$(cut -d' ' -f1 <"$tmp/$name.tar.gz.sha256")
  got=$(file_sha256 "$tmp/$name.tar.gz")
  if [ -z "$want" ] || [ "$want" != "$got" ]; then
    err "checksum mismatch for $name.tar.gz: expected $want, got $got"
  fi
  say "verified SHA-256 $got"

  tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
  [ -f "$tmp/$name/celestia-rs" ] || err "$name.tar.gz has no $name/celestia-rs"
  chmod 755 "$tmp/$name/celestia-rs"
  if ! "$tmp/$name/celestia-rs" version >/dev/null 2>&1; then
    err "the downloaded binary does not run on this machine:
$("$tmp/$name/celestia-rs" version 2>&1 | head -n 5)
${libc_note:+$libc_note
}Build from source instead: https://github.com/$REPO#getting-started"
  fi

  mkdir -p "$bin_dir"
  # The replaced binary stays as celestia-rs.prev, for a rollback (the
  # same name packaging/macos/celestia-rs-auto-update.sh uses).
  replaced=0
  if [ -e "$bin_dir/celestia-rs" ]; then
    replaced=1
    cp -p "$bin_dir/celestia-rs" "$bin_dir/celestia-rs.prev"
  fi
  # Copy next to the destination, then rename: atomic, and a running
  # process keeps its old inode.
  cp "$tmp/$name/celestia-rs" "$bin_dir/.celestia-rs.new"
  mv -f "$bin_dir/.celestia-rs.new" "$bin_dir/celestia-rs"
  printf '%s %s\n' "$1" "$(file_sha256 "$bin_dir/celestia-rs")" >"$home/.install-receipt"
}

# Prints the release tag this installer last installed, if the binary in
# $bin_dir is still that one. The binary itself can't say: its version is
# the crate version, not the release tag.
installed_tag() {
  [ -f "$home/.install-receipt" ] && [ -f "$bin_dir/celestia-rs" ] || return 0
  read -r tag sum <"$home/.install-receipt" || true
  if [ -n "${sum:-}" ] && [ "$sum" = "$(file_sha256 "$bin_dir/celestia-rs")" ]; then
    echo "$tag"
  fi
}

note_running_nodes() {
  if has pgrep && pgrep -x celestia-rs >/dev/null 2>&1; then
    say "a celestia-rs process is running; it keeps the old binary until it restarts"
  fi
}

# Shell snippets that put $bin_dir on PATH, like rustup's ~/.cargo/env.
write_env_files() {
  mkdir -p "$home"
  cat >"$home/env" <<EOF
#!/bin/sh
# celestia-rs shell setup, written by the celestia-rs installer.
case ":\${PATH}:" in
  *:"$bin_dir":*) ;;
  *) export PATH="$bin_dir:\$PATH" ;;
esac
EOF
  cat >"$home/env.fish" <<EOF
# celestia-rs shell setup, written by the celestia-rs installer.
if not contains "$bin_dir" \$PATH
    set -gx PATH "$bin_dir" \$PATH
end
EOF
}

# The line added to POSIX shell profiles, with $HOME left unexpanded when
# the install root is under it.
source_line() {
  # shellcheck disable=SC2016 # $HOME is for the profile to expand
  case $home in
    "$HOME"/*) printf '. "$HOME/%s/env"' "${home#"$HOME"/}" ;;
    *) printf '. "%s/env"' "$home" ;;
  esac
}

# Which profiles to touch follows rustup: ~/.profile always, bash's rc files
# when bash is the login shell or they exist, zsh's .zshenv when zsh is the
# login shell or installed, and a fish conf.d file when fish is.
modify_profiles() {
  line=$(source_line)
  login_shell=$(basename "${SHELL:-sh}")
  # Positional parameters, not a word-split string: zsh doesn't split.
  set -- "$HOME/.profile"
  if [ "$login_shell" = bash ] || [ -f "$HOME/.bashrc" ]; then
    set -- "$@" "$HOME/.bashrc"
  fi
  if [ -f "$HOME/.bash_profile" ]; then
    set -- "$@" "$HOME/.bash_profile"
  elif [ "$login_shell" = bash ] && [ "$(uname -s)" = Darwin ]; then
    # macOS Terminal starts bash as a login shell, which reads
    # .bash_profile and skips .bashrc.
    set -- "$@" "$HOME/.bash_profile"
  fi
  if [ "$login_shell" = zsh ] || has zsh; then
    set -- "$@" "${ZDOTDIR:-$HOME}/.zshenv"
  fi
  changed=""
  for f in "$@"; do
    if [ -f "$f" ] && grep -qF "$line" "$f"; then
      continue
    fi
    if [ -s "$f" ] && [ -n "$(tail -c 1 "$f")" ]; then
      echo >>"$f"
    fi
    echo "$line" >>"$f"
    changed="$changed $f"
  done
  if [ "$login_shell" = fish ] || has fish; then
    fish_dir=${XDG_CONFIG_HOME:-$HOME/.config}/fish/conf.d
    mkdir -p "$fish_dir"
    fish_line="source \"$home/env.fish\""
    if ! { [ -f "$fish_dir/celestia-rs.fish" ] && grep -qF "$fish_line" "$fish_dir/celestia-rs.fish"; }; then
      echo "$fish_line" >"$fish_dir/celestia-rs.fish"
      changed="$changed $fish_dir/celestia-rs.fish"
    fi
  fi
  if [ -n "$changed" ]; then
    say "added $bin_dir to PATH in:$changed"
  fi
}

finish_message() {
  found=$(command -v celestia-rs 2>/dev/null || true)
  if [ -n "$found" ] && [ "$found" != "$bin_dir/celestia-rs" ]; then
    warn "'celestia-rs' on your PATH is $found, which shadows $bin_dir/celestia-rs; remove it or put $bin_dir first"
  fi
  echo
  echo "celestia-rs $version is installed."
  case ":$PATH:" in
    *:"$bin_dir":*) ;;
    *)
      echo
      echo "To use it in this shell, run:"
      echo
      echo "    . \"$home/env\"            # sh, bash, zsh"
      echo "    source \"$home/env.fish\"  # fish"
      echo
      echo "New shells pick it up automatically."
      if [ "$modify_path" = 0 ]; then
        echo "(--no-modify-path: add $bin_dir to PATH yourself.)"
      fi
      ;;
  esac
  echo
  echo "Get started:  celestia-rs version --long"
  echo "Run a node:   https://github.com/$REPO/blob/main/docs/operator-guide.md"
  echo "Update later: re-run this installer."
}

do_uninstall() {
  removed=""
  for f in "$bin_dir/celestia-rs" "$bin_dir/celestia-rs.prev" "$home/.install-receipt" "$home/env" "$home/env.fish" \
    "${XDG_CONFIG_HOME:-$HOME/.config}/fish/conf.d/celestia-rs.fish"; do
    if [ -e "$f" ]; then
      rm -f "$f"
      removed="$removed $f"
    fi
  done
  rmdir "$bin_dir" 2>/dev/null || true
  line=$(source_line)
  for f in "$HOME/.profile" "$HOME/.bashrc" "$HOME/.bash_profile" "${ZDOTDIR:-$HOME}/.zshenv"; do
    if [ -f "$f" ] && grep -qxF "$line" "$f"; then
      # Rewrite in place (not mv) to keep the file's mode, owner and links.
      kept=$(grep -vxF "$line" "$f" || true)
      printf '%s\n' "$kept" >"$f"
      removed="$removed $f(PATH line)"
    fi
  done
  if [ -n "$removed" ]; then
    say "removed:$removed"
  else
    say "nothing to remove under $home"
  fi
  say "left $home in place: it may hold node data (the default --home)"
}

main "$@"
