# Bastio for VS Code

Syntax highlighting for [Bastio](https://github.com/yukikotani231/bastiolang), and the compiler's error shown in the editor when a file is saved.

## Install

The extension is not on the Marketplace yet. Build a package and install it:

```sh
cd editors/vscode
npx @vscode/vsce package          # writes bastio-0.1.0.vsix
code --install-extension bastio-0.1.0.vsix
```

Or, while working on the extension, open `editors/vscode` in VS Code and press F5.

## Diagnostics

On save, the extension runs `bastioc0 --check` on the file and shows the error it reports, with its code (`E-LINEAR-LEAK`, `E-VERIFY-BOUNDS`, ...) and any note. This is the whole pipeline, proofs included, so it needs `z3` on `PATH` like the compiler does. stage0 stops at the first error, so there is at most one at a time. The command **Bastio: Check This File** runs the same check by hand.

Settings:

| Setting | Default | Meaning |
| --- | --- | --- |
| `bastio.compiler.path` | `bastioc0` | The compiler. A relative path is resolved against the workspace folder, e.g. `stage0/target/release/bastioc0` |
| `bastio.compiler.args` | `["--check"]` | Arguments before the file name, e.g. `["--check", "--target", "kernel"]` |
| `bastio.compiler.entry` | (empty) | Check this file instead of the one saved: the program's main file, which pulls the others in with `import` |
| `bastio.checkOnSave` | `true` | Run the check on save |

A file without `fn main` is a library, and checking it alone only says that it has no `main`; that error is not shown. To check a library in the context of a program, set `bastio.compiler.entry` to the program's main file. For BastiOS, for example:

```json
{
  "bastio.compiler.path": "../bastiolang/stage0/target/release/bastioc0",
  "bastio.compiler.args": ["--check", "--target", "kernel"],
  "bastio.compiler.entry": "src/main.bastio"
}
```

(`--check --target kernel` reads the user programs that the kernel embeds with `include_bytes`, so run `make` once first.)

## Tests

```sh
npm ci && npm test
```

tokenizes every `.bastio` file in the repository with the grammar, failing on an `invalid` token, and checks the scopes of a few tricky lines.
