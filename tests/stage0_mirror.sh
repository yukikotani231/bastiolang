#!/usr/bin/env bash
# The stage0 freeze guard (ROADMAP v2, "stage0 is frozen"; docs/bootstrap.md).
# stage1 is the compiler that changes. stage0 stays only to bootstrap
# bastioc1 and to be the oracle stage1/difftest.sh compares against, so every
# change to stage0 (anything under stage0/ that git tracks) must say which
# of the two kinds it is, in a line of one of the branch's commit messages
# (any case):
#
#   Stage0-only: <why>
#     The change has no stage1 counterpart: a bootstrap fix, the native
#     backend (x86.rs, whose stage1 version deliberately differs), comments
#     or formatting, a dependency.
#
#   Stage0-shared: <why>
#     The same change is made in stage1 in the same branch, so the oracle
#     keeps up: a fix to a bug both compilers share (soundness first), or a
#     builtin row BastiOS needs while its kernel is still in difftest's
#     corpus. Each changed stage0/src/*.rs must then come with a change to
#     the stage1 file that mirrors it.
#
# A stage0 change with neither line is a new feature landing in stage0, and
# fails: features land in stage1 only, with test programs marked
# `// stage1-only: <feature>` (tests/stage1_only.sh). An unmirrored change
# with only a Stage0-shared line fails too: its mirror is missing. The
# reasons are for reviewers; the guard only checks that they are there.
# stage1/difftest.sh still compares what the shared changes do.
#
# usage: tests/stage0_mirror.sh [BASE]
#   Compares HEAD with its merge base with BASE (default: origin/main).
set -euo pipefail
cd "$(dirname "$0")/.."

base=${1:-origin/main}
if ! git rev-parse -q --verify "$base^{commit}" >/dev/null; then
  echo "stage0_mirror: no commit '$base' to compare with" >&2
  exit 2
fi
from=$(git merge-base "$base" HEAD)

# stage0 file -> the stage1 files that mirror it (any one of them counts).
declare -A mirror=(
  [lexer.rs]="lexer.bastio"
  [parser.rs]="parser.bastio"
  [ast.rs]="parser.bastio print.bastio render.bastio mono.bastio"
  [mono.rs]="mono.bastio loader.bastio"
  [check.rs]="check.bastio loader.bastio"
  [verify.rs]="verify.bastio"
  [smt.rs]="solve.bastio"
  [codegen.rs]="codegen.bastio cprelude.bastio header.bastio"
  [x86.rs]="x86.bastio"
  [main.rs]="bastioc1.bastio loader.bastio solve.bastio header.bastio"
  [diag.rs]="bastioc1.bastio"
  [dump.rs]="print.bastio lexdump.bastio astdump.bastio monodump.bastio checkdump.bastio smtdump.bastio verifydump.bastio cdump.bastio asmdump.bastio"
)

changed=$(git diff --name-only "$from" HEAD)
touched=()
missing=()
while IFS= read -r f; do
  case $f in stage0/*) touched+=("$f") ;; *) continue ;; esac
  case $f in stage0/src/*.rs) ;; *) missing+=("$f (outside the compiler's source)"); continue ;; esac
  name=${f#stage0/src/}
  if [ -z "${mirror[$name]+set}" ]; then
    missing+=("$f (new file: add it to the map in tests/stage0_mirror.sh)")
    continue
  fi
  ok=
  for m in ${mirror[$name]}; do
    if grep -qx "stage1/$m" <<<"$changed"; then ok=1; break; fi
  done
  [ -n "$ok" ] || missing+=("$f (mirror in stage1/: ${mirror[$name]// /, })")
done <<<"$changed"

if [ ${#touched[@]} -eq 0 ]; then
  echo "stage0_mirror: stage0 is unchanged"
  exit 0
fi
log=$(git log --format=%B "$from..HEAD")
only=$(grep -i '^Stage0-only: *[^ ]' <<<"$log" || true)
shared=$(grep -i '^Stage0-shared: *[^ ]' <<<"$log" || true)
if [ -n "$only" ]; then
  echo "stage0_mirror: stage0 changed, as a commit message says:"
  { echo "$only"; [ -z "$shared" ] || echo "$shared"; } | sed 's/^/  /'
  [ ${#missing[@]} -eq 0 ] || printf '  unmirrored: %s\n' "${missing[@]}"
  exit 0
fi
if [ -n "$shared" ] && [ ${#missing[@]} -eq 0 ]; then
  echo "stage0_mirror: stage0 changed together with stage1, as a commit message says:"
  echo "$shared" | sed 's/^/  /'
  exit 0
fi
if [ -n "$shared" ]; then
  echo "stage0_mirror: a Stage0-shared change is missing its stage1 counterpart:" >&2
  printf '  %s\n' "${missing[@]}" >&2
  cat >&2 <<'EOT'
Make the same change in stage1, or, for the parts that have no stage1
counterpart, say why in a commit message line:
  Stage0-only: <why>
EOT
  exit 1
fi
echo "stage0_mirror: stage0 is frozen, and this branch changes it without saying why:" >&2
printf '  %s\n' "${touched[@]}" >&2
cat >&2 <<'EOT'
New features land in stage1 only, with their test programs marked
`// stage1-only: <feature>` (ROADMAP v2, "stage0 is frozen"; docs/bootstrap.md).
If this change belongs in stage0, say which kind it is in a commit message line:
  Stage0-only: <why>      no stage1 counterpart (bootstrap fix, native backend, comments)
  Stage0-shared: <why>    the same change is made in stage1 (a shared bug fix)
EOT
exit 1
