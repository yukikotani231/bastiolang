# The documentation website

`site/build.sh` builds the website into `site/build/html` with
[mdBook](https://rust-lang.github.io/mdBook/) (CI uses 0.4.52; install it with
`cargo install mdbook --version 0.4.52` or from its release page).

The pages are the repository's own Markdown files (`docs/`, `README.md`,
`ROADMAP.md`, `CONTRIBUTING.md`, `CHANGELOG.md`, the example READMEs), copied
in at build time by `stage.py`, which also turns their relative links into
links between pages or to files on GitHub. Only the table of contents
(`SUMMARY.md`) and the pages that exist only on the site (`bastios.md`,
`c-libraries.md`) live here. To add a page, add it to `PAGES` in `stage.py`
and to `SUMMARY.md`.

After building, `linkcheck.py` fails the build if a link between pages points
at a missing page or anchor. CI runs `site/build.sh` and keeps the HTML as an
artifact (`site-<sha>`).

## Publishing

The site is meant for GitHub Pages, at
<https://yukikotani231.github.io/bastiolang/> (`site-url` in `book.toml`
must match the path if that changes, for example with a custom domain).
There are two ways to publish; use one.

**With GitHub Actions** (preferred: every push to `main` publishes):

1. Settings > Pages > Build and deployment > Source: **GitHub Actions**.
2. Run the `pages` workflow once from the Actions tab (or push to `main`).
   `.github/workflows/pages.yml` builds the site with `site/build.sh` and
   deploys it.

**Without GitHub Actions** (a branch that holds the built HTML):

1. Run `site/publish.sh` from a clone that can push. It builds the site and
   force-pushes the HTML, as a single commit with a `.nojekyll` file, to the
   `gh-pages` branch. Nothing else is touched.
2. Settings > Pages > Build and deployment > Source: **Deploy from a
   branch**, branch `gh-pages`, folder `/ (root)`.
3. Run `site/publish.sh` again whenever the site should be updated.

To check the result locally first: `site/build.sh`, then
`python3 -m http.server -d site/build/html` and open
<http://localhost:8000/>.
