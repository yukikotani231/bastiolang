# Launch checklist

What remained before bastiolang and BastiOS went public, and what is
still open now that they are (both public since 2026-10-11). Tick items
off here as they land, and delete this file once nothing is open. It is not part of the documentation website.

## Decisions for the maintainer

- [x] **Make the repositories public** (2026-10-11), bastiolang first.
      The `BASTIOLANG_TOKEN` gate in bastios' CI and release workflows
      goes in bastios #207.
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
- [x] **The name.** Checked 2026-10-11: `bastio`, `bastios`, `bastioc`
      and `tree-sitter-bastio` are free on crates.io, npm and PyPI, and a
      web search finds no other language or OS by either name. One
      collision: **Bastio** (bastio.com, `bastio-ai` on GitHub) is an AI
      security company with an open-source gateway. Different field, but
      both are security software; whether it holds a registered trademark
      was not checked (USPTO/EUIPO search by hand). "BastiOS" found no
      match. Kept (2026-10-11).

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
      `git log -p --all` for common token and key formats found none;
      gitleaks 8.21.2 over every branch and pull request head of both
      repositories found none on 2026-10-11).
- [ ] **What becomes readable.** All commits and pull requests go public,
      not just `main`. Authors are GitHub noreply addresses and
      `noreply@anthropic.com`; no personal address appears. Most commit
      messages end with a `Claude-Session: https://claude.ai/code/session_...`
      line, and pull request bodies link the claude.ai project thread.
      Both links need the maintainer's login, so they expose no content,
      only that the work was done with Claude Code. One commit message in
      bastios names a local path (`/mnt/project-files/...`), harmless.
      Rewriting history to remove these is not proposed.
- [x] `SECURITY.md` in both repositories (private reporting through
      GitHub security advisories).
- [x] **Turn on private vulnerability reporting** in both repositories
      once public (Settings → Security → Private vulnerability reporting →
      Enable); `SECURITY.md` links to it.
- [x] **Actions on pull requests from outside contributors.** Once public,
      Actions minutes are free and anyone can open a pull request that
      runs our workflows. In each repository: Settings → Actions →
      General → "Approval for running fork pull request workflows from
      contributors" → **Require approval for all external contributors**.
      On the same page keep "Workflow permissions" at **Read repository
      contents** and leave "Allow GitHub Actions to create and approve pull
      requests" off. Workflows use `pull_request`, not
      `pull_request_target`, so fork runs get no secrets.
- [ ] Issue and pull request templates; a `CODE_OF_CONDUCT.md` if wanted.
- [ ] Repository description, topics and website link on GitHub.
- [ ] Branch protection on `main`: require CI.

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
      check the artifacts.
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
