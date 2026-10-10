# Sourced by tests/run.sh and tests/native.sh, after BASTIOC is set.
#
# A test program that uses a feature only stage1 has (ROADMAP v2, "the
# freeze") carries a header line
#
#   // stage1-only: <the feature>
#
# stage0 has no answer for it, so it is skipped when BASTIOC is stage0, and
# stage1/difftest.sh leaves it out of the corpus it compares (checking
# instead that stage0 really rejects it, so a stale marker is caught).

# True if file $1 is marked stage1-only.
stage1_only() { grep -q '^// stage1-only:' "$1"; }

# Set when BASTIOC is stage0, found by how it reports an unknown option, so
# a wrapper script around either compiler is recognised too.
stage0_compiler=
if "$BASTIOC" --no-such-option 2>&1 | grep -q '^bastioc0:'; then
  stage0_compiler=1
fi
