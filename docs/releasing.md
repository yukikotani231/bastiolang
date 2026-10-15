# Releasing

How a version of Bastio is cut. Nothing is released yet; this is the
procedure the release workflow is written for.

## Versions

Bastio uses [semantic versioning](https://semver.org/). Before 1.0, a minor
version (0.2.0 after 0.1.x) may change the language incompatibly, and a
patch version (0.1.1) only fixes bugs. The version lives in one place,
`stage0/Cargo.toml`; the release workflow refuses a tag that does not match
it.

BastiOS is versioned separately, in its own repository.

## Cutting a release

1. On `main`, move the entries under `## [Unreleased]` in `CHANGELOG.md` into
   a new `## [X.Y.Z] - YYYY-MM-DD` section, update the comparison links at
   the bottom, and set `version` in `stage0/Cargo.toml` (then
   `cargo build` so `Cargo.lock` follows).
2. Run `tests/ci.sh`, open a pull request, and merge it once CI is green.
3. Tag the merge commit and push the tag:
   ```sh
   git tag -a vX.Y.Z -m "Bastio X.Y.Z"
   git push origin vX.Y.Z
   ```
4. `.github/workflows/release.yml` runs: it checks that the tag, the crate
   version and the changelog agree, runs the tests, builds `bastioc0` and
   `bastioc1` (bastioc1 built by itself), builds the documentation site,
   packages them (for x86-64 and AArch64, each built and tested on a runner
   of its own architecture), smoke-tests the packages, records a build
   provenance attestation for each archive, and creates a **draft** GitHub
   release with the archives and `SHA256SUMS` attached and the changelog
   section as its notes.
5. Read the draft, then publish it by hand.

To try the build without making a release, start the workflow from the
Actions tab with a tag that already exists: it uploads the archives as a
workflow artifact and stops there.

## What a release contains

- `bastio-vX.Y.Z-x86_64-linux.tar.gz` and `bastio-vX.Y.Z-aarch64-linux.tar.gz`:
  `bin/bastioc0` and `bin/bastioc1`, the examples, the README, the changelog
  and both licenses. The binaries are built on Ubuntu 24.04 and link against
  its glibc. Both compilers need `z3` and a C compiler (`cc`, or `$CC` for
  bastioc0) at run time.
- `bastio-vX.Y.Z-docs.tar.gz`: the documentation website, as static HTML.
- `SHA256SUMS`.

## Checking a download

```sh
sha256sum -c --ignore-missing SHA256SUMS
gh attestation verify bastio-vX.Y.Z-x86_64-linux.tar.gz --repo yukikotani231/bastiolang
```

The attestation says which workflow run, on which commit, built the archive.

## Rebuilding a release

The inputs are fixed so that a rebuild can be compared with a release:

- **Source**: the tag.
- **Rust**: `rust-toolchain.toml` pins the compiler; the build uses
  `--locked`, so `Cargo.lock` decides the dependencies.
- **Paths**: the release workflow builds stage0 with
  `--remap-path-prefix` for the checkout and for `~/.cargo`, so the binary
  does not carry the directory it was built in. Building stage0 twice, from
  two different directories, with the flags in the workflow gives
  byte-identical `bastioc0` (checked on x86-64 Linux).
- **Archives**: files are added in name order with owner 0 and the time of
  the tagged commit (`SOURCE_DATE_EPOCH`), and gzip leaves out its own time
  stamp, so the same files always give the same archive.
- **`bastioc1`** is C compiled by the system `cc`, so its bytes follow the
  C compiler: the same compiler version and the same Z3 give the same file
  (the workflow's runner image is Ubuntu 24.04); another compiler gives a
  working but different binary. Compare a rebuild of `bastioc1` with a
  release only on the same Ubuntu release.
- **Z3** is not part of the archive; the compilers use the one on the user's
  machine. Tested versions are in the README.
