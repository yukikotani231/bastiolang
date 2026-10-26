# Packages and projects

A Bastio program of more than a few files is a *project*: a directory with a `bastio.toml` that names it, lists the other projects (*packages*) it uses, and says where the code is. The `bastio` tool reads the manifest, finds every package, and runs `bastioc1` with the right search paths. Nothing here changes the language: a package is a directory of ordinary `.bastio` files, and `import` is the same `import` as in [Several files](language.md#programs-and-files).

This page says what exists, why it is shaped this way, and what does not exist yet.

## Using it

A project:

```text
app/
  bastio.toml
  src/main.bastio
  src/helper.bastio
```

```toml
# app/bastio.toml
[package]
name = "app"
version = "0.1.0"
entry = "src/main.bastio"      # optional; this is the default

[dependencies]
util = { path = "../util", version = "0.2" }
mathx = { git = "https://example.org/mathx.git", rev = "v1.2.0", version = "1.2" }
```

The code of a package is its `src/` directory. In `app/src/main.bastio`:

```text
import "util/math.bastio";     // util's src/math.bastio
import "./helper.bastio";      // a file of this package, as before
```

An import whose first part is a package's name is that package's `src/` directory. The project's own name works too (`import "app/helper.bastio"`).

```sh
bastio build app               # proves, then builds app/target/app
bastio build app -o /tmp/app -- --backend native
bastio check app               # resolves and proves, builds nothing
bastio update app              # resolves again, ignoring bastio.lock
```

Everything after `--` goes to `bastioc1` unchanged, so `--target`, `--backend` and the others work as usual. `bastio` finds the compiler in `$BASTIOC1`, then as `bastioc1` next to itself, then on `PATH`. Build it with `bastioc1 -o bastio stage1/bastio.bastio`.

Because a dependency is part of the program, the compiler proves it with the rest. An `ensures` of `util` is a fact in `app`, and a call from `app` that breaks a `requires` of `util` is rejected, exactly as it would be inside one package (`tests/package.sh` has the case).

### The manifest

A small TOML subset: comments, the sections `[package]` and `[dependencies]`, and values that are strings. A dependency is an inline table with these keys:

| Key | Meaning |
| --- | --- |
| `path` | A directory with a `bastio.toml`, relative to the manifest that names it |
| `git` | A repository address (anything `git clone` takes) |
| `rev` | With `git`: a tag or a commit id. Not a branch: a branch is not a fixed version |
| `version` | Optional. The requirement `MAJOR[.MINOR[.PATCH]]` |

Exactly one of `path` and `git` is given. A package satisfies a requirement when it has the same major version (the same minor version while the major version is 0) and is not older. The package's own `version` is in its `[package]` section.

The key under `[dependencies]` is the name the package is imported by. It must equal the `name` in that package's own manifest, so a package has one name everywhere.

### Git dependencies and `bastio.lock`

A git dependency is cloned into `$BASTIO_HOME/git/` (default `~/.bastio/git/`) and checked out at `rev`. The first build writes `bastio.lock` next to the manifest, one line per git dependency:

```text
git mathx https://example.org/mathx.git v1.2.0 5cd8cd2ffa057195dffd5bf57546e761ad263aa2
```

The line is the address, the `rev` as written, and the commit it resolved to. Later builds check out that commit, so a moved tag changes nothing until `bastio update` resolves the revs again and rewrites the file. Commit `bastio.lock` with the project. Path dependencies are not locked: they are the working tree.

### Rules

- One flat namespace per project: two different packages with one name, even deep in the tree, are an error that names both directories. The same directory reached twice (a shared dependency, or a cycle) is one package.
- A package's `[package] name` is required; `version` is needed only if someone requires one.
- Dependencies nest at most 32 deep.

## Design

**Why a separate tool.** `bastioc1` stays a compiler: it takes files and search paths. It gained one option, `--pkg NAME=DIR`, which makes `import "NAME/rest"` the file `rest` in `DIR` before any other place is searched. A project is then a way to compute that list of options. Anyone can run `bastioc1 --pkg util=../util/src app/src/main.bastio` without the tool, and the tool's job (reading a manifest, running `git`) is not part of what the compiler's proofs and bootstrap have to carry. The tool is an ordinary hosted Bastio program, proved like any other.

**Why `--lib` was not enough.** `--lib DIR` and the walk up the directories look files up by path alone. They cannot say that `util` is a directory somewhere else, and two libraries with a file of the same name shadow each other. A name that maps to a directory, checked first, fixes both, and keeps `core/` and `std/` working as before: they are found by the old search, unless a project names a package `core`.

**Why `src/`, and a name in the import.** The name in the import is what the dependent wrote in its manifest, not a path, so moving a dependency changes one line of `bastio.toml` and no source.

**Why git pins by commit, and no registry.** A project built from a commit and a lock file builds the same programs later, without a service that can disappear or change. Hosting a registry is a question of people and trust, not of the compiler; the manifest format leaves room for `version`-only dependencies when there is one.

## What is not done

- Versions are checked, not solved: each dependency has one requirement, and when two packages want different versions of a third, the first one resolved wins (or the build fails on a name clash for different directories). There is no version selection from several candidates.
- A registry, publishing, and checksums of fetched sources.
- A versioned standard library. `core/` and `std/` are found next to the compiler (by `--lib` or by walking up from the project); they are not yet a dependency with a version of their own.
- Features, dev-dependencies, workspaces, build scripts.
- Incremental builds: a build proves the whole program each time (`BASTIO_PROOF_CACHE` helps).
