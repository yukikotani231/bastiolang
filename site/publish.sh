#!/usr/bin/env bash
# Publishes the documentation website without GitHub Actions: builds it and
# commits the HTML, as the only commit, to the gh-pages branch of the remote,
# for GitHub Pages' "Deploy from a branch" (branch gh-pages, folder /).
#
# usage: site/publish.sh [REMOTE]      (default: origin)
#
# It replaces whatever gh-pages held (a force push of a branch that holds
# only generated files); main and the working tree are not touched.
set -euo pipefail
cd "$(dirname "$0")/.."
remote=${1:-origin}

site/build.sh
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
cp -r site/build/html/. "$out"
# Not a Jekyll site: serve files whose names start with _ as they are.
touch "$out/.nojekyll"

rev=$(git rev-parse --short HEAD)
git -C "$out" init -q -b gh-pages
git -C "$out" add -A
git -C "$out" -c user.name="$(git config user.name)" -c user.email="$(git config user.email)" \
  commit -q -m "Documentation website built from $rev"
git -C "$out" push -q -f "$(git remote get-url "$remote")" gh-pages
echo "site: published $rev to gh-pages on $remote"
