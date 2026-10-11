# Launch checklist

What remains before bastiolang and BastiOS go public. Written 2026-10-10;
tick items off here as they land, and delete this file once both
repositories are public. It is not part of the documentation website.

## Decisions for the maintainer

- [ ] **Make the repositories public**, bastiolang first: BastiOS CI checks
      bastiolang out, and the `BASTIOLANG_TOKEN` gate in bastios'
      `ci.yml` and `release.yml` can go once bastiolang is public.
- [ ] **The first versions.** Proposed: bastiolang `v0.1.0` (the version in
      `stage0/Cargo.toml`) and BastiOS `v0.2.0` (the version the kernel
      prints). Both changelogs have their section ready, marked "not yet
      tagged".
- [x] **Where the website lives:** GitHub Pages (decided 2026-10-11).
      Both ways to publish are ready ("Publishing" in `site/README.md`):
      the `pages` workflow, off until `PAGES_ENABLED` is set, or
      `site/publish.sh` to a `gh-pages` branch. Turn one on when the
      repository goes public (Pages on a private repository needs a paid
      plan). A custom domain, if any, is still open.
- [ ] **The name.** Check that "Bastio" and "BastiOS" are free to use
      (existing projects, trademarks, package names such as crates.io or
      the VS Code marketplace if the extension is ever published).
- [ ] **fz.** The README calls Bastio "the successor to fz/fzl"; fz is a
      private repository. Either make it public (archived), link nothing,
      or drop the mention.

## Licensing

- [x] Apache-2.0 OR MIT, with `LICENSE-APACHE` and `LICENSE-MIT`, in both
      repositories.
- [x] Commit authors use GitHub noreply addresses only (checked with
      `git log --format='%ae' | sort -u`).
- [x] **GPL in the BastiOS ISO.** The kernel image embeds a BusyBox binary
      (GPL-2.0-only); the release workflow ships its source and
      configuration next to the ISO. The boot loader is Limine
      (BSD-2-Clause) instead of GRUB (GPL-3.0) since 2026-10-11, so no
      GRUB source has to ship: Limine's license is copied into the ISO and
      the USB image as `boot/limine/LICENSE`.
- [ ] **Third-party notices.** musl (MIT) is statically linked into
      BusyBox and the musl test programs in the image, and Limine
      (BSD-2-Clause) is in the ISO; their copyright notices should ship
      with the release too (a `THIRD_PARTY.md` listing BusyBox, musl and
      Limine would cover all three).

## Repository hygiene

- [x] No credentials in the history of either repository (a search of
      `git log -p --all` for common token and key formats found none).
- [ ] `SECURITY.md`: how to report a soundness bug (a program the compiler
      accepts that is not memory safe) privately. For a language whose
      point is a safety proof, these are the security bugs.
- [ ] Issue and pull request templates; a `CODE_OF_CONDUCT.md` if wanted.
- [ ] Repository description, topics and website link on GitHub.
- [ ] Branch protection on `main`: require CI.
- [ ] Re-enable or archive the CI of `fz` if it becomes public.

## Documentation drift

Statements that were true when written and are not any more:

- [ ] README.md: "the language BastiOS will be written in" (it is), and the
      layout table calls `docs/concurrency.md` "not implemented yet"
      (locks, atomics and lock order are).
- [ ] docs/integers.md: "Status: design ... Not implemented yet" (all four
      steps are done).
- [ ] ROADMAP.md v4: the 64 MiB user window, signals and blocking `poll`
      are done in BastiOS but still unchecked; TCP for Linux programs
      (v3) likewise. v3 "BastiOS on several cores" is partly done (the
      scheduler runs on every processor under one lock).
- [ ] The tutorial's error transcripts are not compared with the
      compiler's output automatically; re-capture them before the launch.

## Release engineering

- [x] Changelogs (`CHANGELOG.md`) in both repositories.
- [x] Release workflows that build from a tag and create a draft release
      (`docs/releasing.md` in both repositories). Not run yet.
- [ ] Dry run: start each release workflow by hand on a throwaway tag and
      check the artifacts (bastios needs `BASTIOLANG_TOKEN` for this).
- [ ] `bastioc0 --version` does not exist; add it (stage0, owned by the
      compiler work) so bug reports can say which compiler they used.
- [ ] Binaries are built against Ubuntu 24.04's glibc. A static musl
      build of bastioc0 would run on more systems.

## The software itself

Known gaps a newcomer will hit first. Not blockers, but say them up front
(the READMEs mostly do):

- [ ] bastioc1 prints error codes without the sentence after them; stage0
      is the compiler to recommend until it does.
- [ ] BastiOS has only been tried in QEMU, needs a serial console, and runs
      on one processor under UEFI.
- [ ] Only x86-64 Linux hosts are tested.
