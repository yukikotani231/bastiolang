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
artifact (`site-<sha>`); the site is not deployed anywhere yet.
