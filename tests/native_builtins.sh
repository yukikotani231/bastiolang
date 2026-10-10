#!/usr/bin/env bash
# Every builtin the checker knows (a `bsig` or `bsig5` row in
# stage1/check.bastio) must have code in the native backend
# (stage1/x86.bastio names it in quotes), or be listed in
# tests/native_builtins_todo.txt. Without this, a new builtin works with the
# C backend and the native backend only finds out when a program using it
# (BastiOS's kernel, say) stops compiling natively: that is how Shared pages
# went unsupported for a while.
#
# A listed builtin that now has native code fails the check too, so the
# list only ever names what is really missing.
#
# usage: tests/native_builtins.sh
set -euo pipefail
cd "$(dirname "$0")/.."

todo=tests/native_builtins_todo.txt
builtins=$(grep -oE '\bbsig5?\(&mut c, "[a-z0-9_]+"' stage1/check.bastio | sed -E 's/.*"(.*)"/\1/' | sort -u)
if [ -z "$builtins" ]; then
  echo "native_builtins: found no bsig rows in stage1/check.bastio; update this script" >&2
  exit 2
fi
listed=$(grep -vE '^\s*(#|$)' "$todo" | awk '{print $1}' | sort -u || true)

bad=0
n=0
for b in $builtins; do
  n=$((n + 1))
  has=0
  grep -qF "\"$b\"" stage1/x86.bastio && has=1
  if grep -qxF "$b" <<<"$listed"; then
    if [ $has = 1 ]; then
      echo "native_builtins: \`$b\` has native code now; remove it from $todo"
      bad=1
    fi
  elif [ $has = 0 ]; then
    echo "native_builtins: builtin \`$b\` has no code in stage1/x86.bastio"
    echo "  add it there, or list it in $todo with the reason it can wait"
    bad=1
  fi
done
for b in $listed; do
  if ! grep -qxF "$b" <<<"$builtins"; then
    echo "native_builtins: $todo lists \`$b\`, which is not a builtin"
    bad=1
  fi
done
[ $bad = 0 ] && echo "native_builtins: all $n builtins have native code or are listed in $todo"
exit $bad
