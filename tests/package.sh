#!/usr/bin/env bash
# The `bastio` project tool and `bastioc1 --pkg`: a manifest, packages found
# by path and by git, bastio.lock, and the errors for a bad project.
#
# usage: tests/package.sh
set -euo pipefail
cd "$(dirname "$0")/.."
command -v z3 >/dev/null || { echo "package: SKIPPED (no z3)"; exit 0; }
BASTIOC=$(tests/bastioc1.sh) || exit 1
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
bastio="$tmp/bastio"
"$BASTIOC" -o "$bastio" stage1/bastio.bastio >/dev/null
export BASTIOC1="$BASTIOC" BASTIO_HOME="$tmp/home"

fail() { echo "package: FAIL: $*" >&2; exit 1; }
# expect_err DIR MESSAGE: `bastio check DIR` fails and says MESSAGE.
expect_err() {
  if out=$("$bastio" check "$1" 2>&1); then fail "$1 was accepted"; fi
  case $out in *"$2"*) ;; *) fail "$1: expected '$2', got: $out" ;; esac
}

# --- a library next to the program, by path ---------------------------------
mkdir -p "$tmp/app/src" "$tmp/util/src"
cat >"$tmp/util/bastio.toml" <<'T'
[package]
name = "util"
version = "0.2.1"
T
cat >"$tmp/util/src/math.bastio" <<'T'
fn twice(x: u64) -> u64
  requires x < 1000
  ensures result == x + x
{
  return x + x;
}
T
cat >"$tmp/app/bastio.toml" <<'T'
# a comment
[package]
name = "app"
version = "0.1.0"

[dependencies]
util = { path = "../util", version = "0.2" }
T
cat >"$tmp/app/src/helper.bastio" <<'T'
fn helper() -> u64 { return 21; }
T
cat >"$tmp/app/src/main.bastio" <<'T'
import "util/math.bastio";
import "./helper.bastio";
import "app/helper.bastio";
fn main(con: &mut Serial) -> u64 {
  let h = helper();
  if h < 1000 {
    return twice(h);
  }
  return 0;
}
T
"$bastio" build "$tmp/app" >/dev/null
set +e; "$tmp/app/target/app"; st=$?; set -e
[ "$st" = 42 ] || fail "app exited $st, not 42"
[ ! -e "$tmp/app/bastio.lock" ] || fail "path dependencies are not pinned"
"$bastio" build "$tmp/app" -o "$tmp/out" -- --emit-c >/dev/null
[ -s "$tmp/out" ] || fail "-o and options after -- did not reach the compiler"

# A proof that fails across the package boundary is reported.
sed -i 's/if h < 1000 {/if h < 100000 {/' "$tmp/app/src/main.bastio"
expect_err "$tmp/app" "twice"
sed -i 's/if h < 100000 {/if h < 1000 {/' "$tmp/app/src/main.bastio"

# --- errors -----------------------------------------------------------------
cp -r "$tmp/app" "$tmp/bad"
sed -i 's/version = "0.2"/version = "0.3"/' "$tmp/bad/bastio.toml"
expect_err "$tmp/bad" "does not satisfy"
sed -i 's/version = "0.3"/version = "0.1"/' "$tmp/bad/bastio.toml"
expect_err "$tmp/bad" "does not satisfy"
sed -i 's/util = { path = "..\/util"/other = { path = "..\/util"/' "$tmp/bad/bastio.toml"
expect_err "$tmp/bad" "not named what its dependent calls it"
sed -i 's/other = { path = "..\/util".*/util = { path = "..\/nowhere" }/' "$tmp/bad/bastio.toml"
expect_err "$tmp/bad" "cannot read"
sed -i 's/util = .*/util = { path = "..\/util", frob = "x" }/' "$tmp/bad/bastio.toml"
expect_err "$tmp/bad" "unknown key in a dependency"
sed -i 's/util = .*/util = { path = "..\/util", git = "x" }/' "$tmp/bad/bastio.toml"
expect_err "$tmp/bad" "exactly one of"
sed -i 's/util = .*/util = "1.0"/' "$tmp/bad/bastio.toml"
expect_err "$tmp/bad" "bastio.toml:7"
rm "$tmp/bad/bastio.toml"
expect_err "$tmp/bad" "cannot read"

# Two different packages with one name; and a cycle, which is fine.
mkdir -p "$tmp/util2/src" "$tmp/mid/src"
printf '[package]\nname = "util"\nversion = "0.2.0"\n' >"$tmp/util2/bastio.toml"
printf '[package]\nname = "mid"\n\n[dependencies]\nutil = { path = "../util2" }\n' >"$tmp/mid/bastio.toml"
cp -r "$tmp/app" "$tmp/clash"
printf '\nmid = { path = "../mid" }\n' >>"$tmp/clash/bastio.toml"
expect_err "$tmp/clash" "two different packages are called util"
printf '[package]\nname = "util"\nversion = "0.2.1"\n\n[dependencies]\napp = { path = "../app" }\n' >"$tmp/util/bastio.toml"
"$bastio" check "$tmp/app" >/dev/null || fail "a cycle of packages should resolve"

# --- git dependencies, pinned by bastio.lock --------------------------------
if command -v git >/dev/null; then
  mkdir -p "$tmp/mathx/src"
  printf '[package]\nname = "mathx"\nversion = "1.0.3"\n' >"$tmp/mathx/bastio.toml"
  printf 'fn seven() -> u64 { return 7; }\n' >"$tmp/mathx/src/seven.bastio"
  g() { git -C "$tmp/mathx" -c user.email=t@t -c user.name=t "$@"; }
  g init -q -b main . && g add -A && g commit -qm one && g tag v1.0.0
  first=$(g rev-parse HEAD)
  mkdir -p "$tmp/gapp/src"
  cat >"$tmp/gapp/bastio.toml" <<T
[package]
name = "gapp"
version = "0.1.0"

[dependencies]
mathx = { git = "$tmp/mathx", rev = "v1.0.0", version = "1.0" }
T
  printf 'import "mathx/seven.bastio";\nfn main(con: &mut Serial) -> u64 { return seven(); }\n' >"$tmp/gapp/src/main.bastio"
  "$bastio" build "$tmp/gapp" >/dev/null
  set +e; "$tmp/gapp/target/gapp"; st=$?; set -e
  [ "$st" = 7 ] || fail "gapp exited $st, not 7"
  grep -q "^git mathx $tmp/mathx v1.0.0 $first\$" "$tmp/gapp/bastio.lock" || fail "bastio.lock does not pin $first"
  # The tag moves; the lock keeps the old commit until `bastio update`.
  printf 'fn seven() -> u64 { return 8; }\n' >"$tmp/mathx/src/seven.bastio"
  g commit -qam two && g tag -f v1.0.0 >/dev/null
  second=$(g rev-parse HEAD)
  "$bastio" build "$tmp/gapp" >/dev/null
  set +e; "$tmp/gapp/target/gapp"; st=$?; set -e
  [ "$st" = 7 ] || fail "the lock did not hold the old commit (exit $st)"
  "$bastio" update "$tmp/gapp" >/dev/null
  grep -q " $second\$" "$tmp/gapp/bastio.lock" || fail "bastio update did not move the pin to $second"
  "$bastio" build "$tmp/gapp" >/dev/null
  set +e; "$tmp/gapp/target/gapp"; st=$?; set -e
  [ "$st" = 8 ] || fail "after update gapp exited $st, not 8"
  sed -i 's/v1.0.0/v9.9.9/' "$tmp/gapp/bastio.toml"
  expect_err "$tmp/gapp" "cannot fetch the git dependency"
else
  echo "package: git part SKIPPED (no git)"
fi

# --- the compiler's own --pkg ------------------------------------------------
if out=$("$BASTIOC" --check --pkg nodir "$tmp/app/src/main.bastio" 2>&1); then fail "--pkg without = was accepted"; fi
case $out in *"NAME=DIR"*) ;; *) fail "--pkg without =: $out" ;; esac

echo "package: ok"
