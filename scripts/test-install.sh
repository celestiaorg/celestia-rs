#!/bin/sh
# Tests scripts/install.sh against a mock GitHub served from localhost:
# github.com's /releases/latest redirect and download URLs, and the
# api.github.com endpoints the installer uses with a token (private
# repositories). Release archives and their .sha256 files are laid out like
# .github/workflows/release.yml makes them, but the archived "binary" is a
# shell script, so this runs in seconds on any Linux or macOS machine;
# .github/workflows/install.yml runs it across distros, shells and macOS.
#
#   scripts/test-install.sh                 # run the installer under sh
#   TEST_SHELL=dash scripts/test-install.sh # ... or under another shell
#
# Needs python3 (for the server), curl or wget, tar, and the tools the
# installer needs.

set -eu

here=$(cd "$(dirname "$0")" && pwd)
installer=$here/install.sh
shell=${TEST_SHELL:-sh}
work=$(mktemp -d 2>/dev/null || mktemp -d -t celestia-rs-test)
server_pid=""
cleanup() {
  [ -z "$server_pid" ] || kill "$server_pid" 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

fails=0
pass() { printf 'ok   %s\n' "$1"; }
fail() {
  printf 'FAIL %s\n' "$1"
  [ ! -f "$work/out" ] || sed 's/^/     | /' "$work/out"
  fails=$((fails + 1))
}

if command -v sha256sum >/dev/null 2>&1; then
  sha() { sha256sum "$1" | cut -d' ' -f1; }
else
  sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
fi

# --- mock releases -----------------------------------------------------------

# make_release REPO TAG: archives for every release target, plus checksums.
make_release() {
  dir=$work/www/$1/releases/download/$2
  mkdir -p "$dir"
  for target in aarch64-apple-darwin x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
    name=celestia-rs-$2-$target
    mkdir -p "$work/stage/$name"
    # Like the real binary, `version` prints the crate version, not the tag.
    printf '#!/bin/sh\necho 0.0.1\necho "%s" >&2\n' "$2" >"$work/stage/$name/celestia-rs"
    chmod 755 "$work/stage/$name/celestia-rs"
    cp "$here/../LICENSE-MIT" "$work/stage/$name/"
    tar -C "$work/stage" -czf "$dir/$name.tar.gz" "$name"
    (cd "$dir" && printf '%s  %s\n' "$(sha "$name.tar.gz")" "$name.tar.gz" >"$name.tar.gz.sha256")
  done
}

# set_releases REPO "TAG PRERELEASE(0|1)"...: the releases, newest first.
set_releases() {
  repo=$1
  shift
  mkdir -p "$work/www/$repo"
  : >"$work/www/$repo/releases.txt"
  for r in "$@"; do
    echo "$r" >>"$work/www/$repo/releases.txt"
    make_release "$repo" "${r% *}"
  done
}

set_releases mock/stable "v0.3.0-rc.1 1" "v0.2.0 0" "v0.1.0 0"
set_releases mock/pre "v0.1.0-alpha.2 1" "v0.1.0-alpha.1 1"
mkdir -p "$work/www/mock/none" && : >"$work/www/mock/none/releases.txt"
set_releases mock/private "v0.2.0 0" "v0.1.0 0"
: >"$work/www/mock/private/private"
set_releases mock/bad "v9.9.9 0"
for f in "$work"/www/mock/bad/releases/download/v9.9.9/*.sha256; do
  printf '%064d  x\n' 0 >"$f"
done

cat >"$work/server.py" <<'PY'
# A mock of github.com and api.github.com. A repo directory holding a
# `private` file is private: only the API serves it, and only with the
# token "secret". API asset downloads redirect to /storage on another host
# name (localhost, vs 127.0.0.1), which rejects requests that carry a
# token, as GitHub's storage does.
import http.server, json, os, sys

root = sys.argv[1]
TOKEN = "Bearer secret"


def releases(repo):
    path = os.path.join(root, repo, "releases.txt")
    if not os.path.exists(path):
        return None
    out = []
    for line in open(path):
        tag, pre = line.split()
        out.append({
            "tag_name": tag, "prerelease": pre == "1", "draft": False,
            # Commas, braces and an escaped decoy key, to trip naive parsing.
            "body": 'Notes, {with} commas, and \\"tag_name\\": \\"decoy\\"',
        })
    return out


def private(repo):
    return os.path.exists(os.path.join(root, repo, "private"))


def assets(repo):
    """Every (tag, file) of a repo; an asset's id is its index."""
    out = []
    for r in releases(repo) or []:
        d = os.path.join(root, repo, "releases", "download", r["tag_name"])
        out += [(r["tag_name"], n) for n in sorted(os.listdir(d))]
    return out


class H(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **k):
        super().__init__(*a, directory=root, **k)

    def log_message(self, *a):
        pass

    def send(self, code, body=b"", ctype="text/plain", location=None):
        self.send_response(code)
        if location:
            self.send_header("Location", location)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(body)

    def send_json(self, obj):
        # Minified, unlike GitHub's, so the installer can't rely on one key
        # per line.
        return self.send(200, json.dumps(obj, separators=(",", ":")).encode(), "application/json")

    def not_found(self):
        return self.send(404, b'{"message":"Not Found"}', "application/json")

    def do_HEAD(self):
        self.do_GET()

    def do_GET(self):
        host = self.headers.get("Host", "")
        auth = self.headers.get("Authorization")
        p = self.path.split("?")[0].strip("/").split("/")
        if p[0] == "storage":
            if auth:
                return self.send(400, b"Only one auth mechanism allowed")
            path = os.path.join(root, *p[1:])
            if not os.path.isfile(path):
                return self.not_found()
            return self.send(200, open(path, "rb").read(), "application/octet-stream")
        if p[0] == "api" and len(p) >= 5 and p[1] == "repos" and p[4] == "releases":
            return self.api(host, auth, p[2] + "/" + p[3], p[5:])
        repo = "/".join(p[:2])
        if private(repo):
            return self.not_found()
        if p[2:] == ["releases", "latest"]:
            stable = [r for r in (releases(repo) or []) if not r["prerelease"]]
            # With no stable release, GitHub redirects to the releases page.
            loc = "/%s/releases/tag/%s" % (repo, stable[0]["tag_name"]) if stable else "/%s/releases" % repo
            return self.send(302, location=loc)
        if p[2:] == ["releases"] or p[2:4] == ["releases", "tag"]:
            return self.send(200, b"<html>release page</html>", "text/html")
        return super().do_GET()

    def api(self, host, auth, repo, rest):
        if auth and auth != TOKEN:
            return self.send(401, b'{"message":"Bad credentials"}', "application/json")
        rs = releases(repo)
        if rs is None or (private(repo) and auth != TOKEN):
            return self.not_found()
        base = "http://%s/api/repos/%s/releases" % (host, repo)
        if rest == []:
            return self.send_json(rs)
        if rest == ["latest"]:
            stable = [r for r in rs if not r["prerelease"]]
            return self.send_json(stable[0]) if stable else self.not_found()
        if len(rest) == 2 and rest[0] == "tags":
            r = [r for r in rs if r["tag_name"] == rest[1]]
            if not r:
                return self.not_found()
            return self.send_json(dict(r[0], url=base + "/1", assets=[
                {"url": "%s/assets/%d" % (base, i), "id": i, "name": n,
                 "uploader": {"url": "http://%s/api/users/x" % host, "login": "x"}}
                for i, (t, n) in enumerate(assets(repo)) if t == rest[1]]))
        if len(rest) == 2 and rest[0] == "assets":
            if self.headers.get("Accept") != "application/octet-stream":
                return self.send(415, b"want Accept: application/octet-stream")
            all_assets = assets(repo)
            i = int(rest[1])
            if i >= len(all_assets):
                return self.not_found()
            tag, name = all_assets[i]
            port = host.rsplit(":", 1)[1]
            return self.send(302, location="http://localhost:%s/storage/%s/releases/download/%s/%s"
                             % (port, repo, tag, name))
        return self.not_found()


s = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
print(s.server_address[1], flush=True)
s.serve_forever()
PY

python3 -I "$work/server.py" "$work/www" >"$work/port" 2>"$work/server.log" &
server_pid=$!
# Up to a minute: a cold python3 can be slow to start on a fresh runner.
i=0
while [ ! -s "$work/port" ]; do
  i=$((i + 1))
  if [ $i -gt 600 ] || ! kill -0 "$server_pid" 2>/dev/null; then
    echo "the mock server did not start ($(command -v python3), $(python3 --version 2>&1)):"
    cat "$work/server.log"
    exit 1
  fi
  sleep 0.1
done
base=http://127.0.0.1:$(head -n 1 "$work/port")

# Fake GitHub CLIs: one logged in (token "secret"), one logged out.
mkdir -p "$work/gh-in" "$work/gh-out"
# shellcheck disable=SC2016 # expanded by the fake gh
printf '#!/bin/sh\n[ "$1 $2" = "auth token" ] && echo secret\n' >"$work/gh-in/gh"
printf '#!/bin/sh\necho "not logged in" >&2\nexit 1\n' >"$work/gh-out/gh"
chmod 755 "$work/gh-in/gh" "$work/gh-out/gh"

# --- helpers -----------------------------------------------------------------

# A fresh $HOME for each scenario.
new_home() {
  h=$work/home-$1
  rm -rf "$h"
  mkdir -p "$h"
}

# run REPO [ARGS...]: run the installer piped into $shell, as the README
# does. T_TOKEN is the GitHub token to pass (none by default) and T_GH the
# fake gh to put on PATH (in or out; out by default); set them in a
# subshell, `(T_GH=in && run ...)`, since bash 3.2 (macOS sh) keeps an
# assignment written before a function call. Output goes to
# $work/out; the exit status is the installer's.
run() {
  repo=$1
  shift
  set +e
  HOME=$h SHELL=/bin/bash ZDOTDIR="" XDG_CONFIG_HOME="" CELESTIA_RS_HOME="" \
    GITHUB_TOKEN=${T_TOKEN:-} GH_TOKEN="" PATH="$work/gh-${T_GH:-out}:$PATH" \
    CELESTIA_RS_REPO=$repo CELESTIA_RS_GITHUB_URL=$base CELESTIA_RS_GITHUB_API=$base/api \
    $shell -s -- "$@" <"$installer" >"$work/out" 2>&1
  status=$?
  set -e
  return $status
}

installed() { HOME=$h "$h/.celestia-rs/bin/celestia-rs" version 2>&1 | tail -n 1; }
receipt() { cut -d' ' -f1 "$h/.celestia-rs/.install-receipt" 2>/dev/null || true; }
has_out() { grep -qF -- "$1" "$work/out"; }

# --- scenarios ---------------------------------------------------------------

t="fresh install picks the latest stable release"
new_home fresh
if run mock/stable && [ "$(installed)" = v0.2.0 ] && [ "$(receipt)" = v0.2.0 ] &&
  has_out "installed celestia-rs v0.2.0"; then pass "$t"; else fail "$t"; fi

t="profiles source the env file, which puts bin on PATH"
# shellcheck disable=SC2016 # the literal line the installer writes
line='. "$HOME/.celestia-rs/env"'
if grep -qxF "$line" "$h/.profile" && grep -qxF "$line" "$h/.bashrc" &&
  [ "$(HOME=$h PATH=/usr/bin:/bin $shell -c ". \"$h/.celestia-rs/env\"; command -v celestia-rs")" = "$h/.celestia-rs/bin/celestia-rs" ]; then
  pass "$t"
else fail "$t"; fi

t="the env file doesn't add bin to PATH twice"
p=$(HOME=$h PATH=/usr/bin:/bin $shell -c ". \"$h/.celestia-rs/env\"; . \"$h/.celestia-rs/env\"; echo \$PATH")
if [ "$p" = "$h/.celestia-rs/bin:/usr/bin:/bin" ]; then pass "$t"; else
  echo "PATH=$p" >"$work/out"
  fail "$t"
fi

t="re-running is a no-op and doesn't duplicate profile lines"
if run mock/stable && has_out "v0.2.0 is already installed" &&
  [ "$(grep -cxF "$line" "$h/.profile")" = 1 ] && [ "$(grep -cxF "$line" "$h/.bashrc")" = 1 ]; then
  pass "$t"
else fail "$t"; fi

t="--version pins a release and keeps the replaced binary as .prev"
if run mock/stable --version v0.1.0 && [ "$(installed)" = v0.1.0 ] &&
  [ "$(HOME=$h "$h/.celestia-rs/bin/celestia-rs.prev" 2>&1 | tail -n 1)" = v0.2.0 ]; then
  pass "$t"
else fail "$t"; fi

t="--version accepts a tag without the leading v"
if run mock/stable --version=0.2.0 && [ "$(installed)" = v0.2.0 ] &&
  has_out "updated celestia-rs v0.1.0 -> v0.2.0"; then pass "$t"; else fail "$t"; fi

t="--prerelease picks the newest release, pre-releases included"
if run mock/stable --prerelease && [ "$(installed)" = v0.3.0-rc.1 ]; then pass "$t"; else fail "$t"; fi

t="CELESTIA_RS_VERSION works like --version"
(export CELESTIA_RS_VERSION=v0.1.0 && run mock/stable) || true
if [ "$(installed)" = v0.1.0 ]; then pass "$t"; else fail "$t"; fi

t="with no stable release, falls back to the newest pre-release"
new_home pre
if run mock/pre && [ "$(installed)" = v0.1.0-alpha.2 ] && has_out "no stable release yet"; then
  pass "$t"
else fail "$t"; fi

t="a source-built binary already in bin is replaced and kept as .prev"
new_home srcbuild
mkdir -p "$h/.celestia-rs/bin"
printf '#!/bin/sh\necho source-build >&2\n' >"$h/.celestia-rs/bin/celestia-rs"
chmod 755 "$h/.celestia-rs/bin/celestia-rs"
if run mock/stable && [ "$(installed)" = v0.2.0 ] && has_out "the binary it replaced is" &&
  grep -q source-build "$h/.celestia-rs/bin/celestia-rs.prev"; then pass "$t"; else fail "$t"; fi

t="a checksum mismatch aborts and leaves the installed binary alone"
if ! run mock/bad && has_out "checksum mismatch" && [ "$(installed)" = v0.2.0 ] &&
  [ ! -e "$h/.celestia-rs/bin/.celestia-rs.new" ]; then pass "$t"; else fail "$t"; fi

t="an unknown version fails with a clear error"
if ! run mock/stable --version v7.7.7 && has_out "could not download" && [ "$(installed)" = v0.2.0 ]; then
  pass "$t"
else fail "$t"; fi

t="a repo with no releases fails with a clear error"
new_home none
if ! run mock/none && has_out "has no releases yet" && [ ! -e "$h/.celestia-rs/bin/celestia-rs" ]; then
  pass "$t"
else fail "$t"; fi

t="token: installs the latest stable release through the API"
new_home token
if (T_TOKEN=secret && run mock/stable) && [ "$(installed)" = v0.2.0 ]; then pass "$t"; else fail "$t"; fi

t="token: --prerelease and --version work through the API"
if (T_TOKEN=secret && run mock/stable --prerelease) && [ "$(installed)" = v0.3.0-rc.1 ] &&
  (T_TOKEN=secret && run mock/stable --version v0.1.0) && [ "$(installed)" = v0.1.0 ]; then
  pass "$t"
else fail "$t"; fi

t="token: with no stable release, falls back to the newest pre-release"
new_home token-pre
if (T_TOKEN=secret && run mock/pre) && [ "$(installed)" = v0.1.0-alpha.2 ]; then pass "$t"; else fail "$t"; fi

t="private repo: installs with GITHUB_TOKEN"
new_home private
if (T_TOKEN=secret && run mock/private) && [ "$(installed)" = v0.2.0 ]; then pass "$t"; else fail "$t"; fi

t="private repo: falls back to a logged-in gh CLI's token"
new_home private-gh
if (T_GH=in && run mock/private) && [ "$(installed)" = v0.2.0 ] &&
  [ "$(grep -c "using the GitHub CLI's login" "$work/out")" = 1 ]; then
  pass "$t"
else fail "$t"; fi

t="private repo: --version falls back to a logged-in gh CLI's token"
new_home private-gh-version
if (T_GH=in && run mock/private --version v0.1.0) && [ "$(installed)" = v0.1.0 ]; then pass "$t"; else fail "$t"; fi

t="private repo: without credentials, says to set GITHUB_TOKEN"
new_home private-none
if ! run mock/private && has_out "set GITHUB_TOKEN" && [ ! -e "$h/.celestia-rs/bin/celestia-rs" ]; then
  pass "$t"
else fail "$t"; fi

t="a bad token fails without installing"
new_home badtoken
if ! (T_TOKEN=wrong && run mock/private) && [ ! -e "$h/.celestia-rs/bin/celestia-rs" ]; then
  pass "$t"
else fail "$t"; fi

t="--no-modify-path leaves shell profiles alone"
new_home nopath
if run mock/stable --no-modify-path && [ "$(installed)" = v0.2.0 ] && [ ! -e "$h/.profile" ] &&
  [ ! -e "$h/.bashrc" ] && has_out "add $h/.celestia-rs/bin to PATH yourself"; then
  pass "$t"
else fail "$t"; fi

t="an unknown option fails"
if ! run mock/stable --bogus && has_out "unknown option '--bogus'"; then pass "$t"; else fail "$t"; fi

t="--help prints usage"
if run mock/stable --help && has_out "USAGE:"; then pass "$t"; else fail "$t"; fi

t="CELESTIA_RS_HOME moves the install root"
new_home custom
set +e
HOME=$h SHELL=/bin/bash CELESTIA_RS_HOME=$h/opt/crs CELESTIA_RS_REPO=mock/stable \
  GITHUB_TOKEN="" GH_TOKEN="" PATH="$work/gh-out:$PATH" \
  CELESTIA_RS_GITHUB_URL=$base CELESTIA_RS_GITHUB_API=$base/api \
  $shell -s <"$installer" >"$work/out" 2>&1
set -e
if [ -x "$h/opt/crs/bin/celestia-rs" ] && grep -qxF ". \"\$HOME/opt/crs/env\"" "$h/.profile"; then
  pass "$t"
else fail "$t"; fi

t="--uninstall removes the binary and PATH lines but keeps node data"
new_home uninstall
run mock/stable --version v0.1.0 >/dev/null 2>&1 || true
run mock/stable >/dev/null 2>&1 || true
mkdir -p "$h/.celestia-rs/config" && echo keep >"$h/.celestia-rs/config/node_key.json"
echo "alias ll='ls -l'" >>"$h/.bashrc"
if run mock/stable --uninstall && [ ! -e "$h/.celestia-rs/bin" ] && [ ! -e "$h/.celestia-rs/env" ] &&
  [ ! -e "$h/.celestia-rs/.install-receipt" ] && [ -f "$h/.celestia-rs/config/node_key.json" ] &&
  ! grep -qF ".celestia-rs/env" "$h/.profile" "$h/.bashrc" && grep -qF "alias ll" "$h/.bashrc"; then
  pass "$t"
else fail "$t"; fi

if command -v zsh >/dev/null 2>&1; then
  t="zsh: .zshenv sources the env file"
  new_home zsh
  if run mock/stable && grep -qxF "$line" "$h/.zshenv" &&
    [ "$(HOME=$h PATH=/usr/bin:/bin env -u ZDOTDIR zsh -c 'command -v celestia-rs')" = "$h/.celestia-rs/bin/celestia-rs" ]; then
    pass "$t"
  else fail "$t"; fi
fi

if command -v fish >/dev/null 2>&1; then
  t="fish: conf.d puts bin on PATH"
  new_home fish
  if run mock/stable && [ -f "$h/.config/fish/conf.d/celestia-rs.fish" ] &&
    [ "$(HOME=$h XDG_CONFIG_HOME='' PATH=/usr/bin:/bin fish -c 'command -v celestia-rs')" = "$h/.celestia-rs/bin/celestia-rs" ]; then
    pass "$t"
  else fail "$t"; fi
fi

echo
if [ "$fails" -gt 0 ]; then
  echo "$fails scenario(s) failed (installer run with $shell)"
  exit 1
fi
echo "all scenarios passed (installer run with $shell)"
