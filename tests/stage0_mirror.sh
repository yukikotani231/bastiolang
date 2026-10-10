#!/usr/bin/env bash
# The stage0 freeze guard: a change to stage0's compiler (stage0/src/*.rs)
# must come with a change to its stage1 counterpart, because stage1 is the
# compiler that stays (ROADMAP v2, "stage0 is frozen"). stage1/difftest.sh
# catches a mirror that behaves differently on the corpus; this catches the
# mirror nobody wrote, including for behaviour the corpus does not reach.
#
# A change that has no stage1 counterpart (a stage0-only bug in code stage1
# does not share, a comment, a bootstrap fix) says so with a line
#
#   Stage0-only: <why>
#
# in one of the branch's commit messages. The reason is for reviewers; the
# guard only checks that it is there.
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
missing=()
while IFS= read -r f; do
  case $f in stage0/src/*.rs) ;; *) continue ;; esac
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

if [ ${#missing[@]} -eq 0 ]; then
  echo "stage0_mirror: every stage0 change has a stage1 counterpart"
  exit 0
fi
if git log --format=%B "$from..HEAD" | grep -q '^Stage0-only: *[^ ]'; then
  echo "stage0_mirror: stage0-only change, as a commit message says:"
  git log --format=%B "$from..HEAD" | grep '^Stage0-only:' | sed 's/^/  /'
  printf '  unmirrored: %s\n' "${missing[@]}"
  exit 0
fi
echo "stage0_mirror: stage0 changed without its stage1 counterpart:" >&2
printf '  %s\n' "${missing[@]}" >&2
cat >&2 <<'EOF'
Mirror the change in stage1 (CLAUDE.md, ROADMAP v2 "stage0 is frozen"), or,
if it has no stage1 counterpart, say why in a commit message line:
  Stage0-only: <why>
EOF
exit 1
