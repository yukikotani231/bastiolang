# Editor support

- [`vscode/`](vscode/): a VS Code extension. Syntax highlighting (a TextMate grammar) and the compiler's error shown in the editor on save.
- [`tree-sitter-bastio/`](tree-sitter-bastio/): a tree-sitter grammar with highlight and fold queries, for Neovim, Helix, Zed and other editors built on tree-sitter.

- [`vim/`](vim/README.md): Vim and Neovim: file type, syntax, indentation, and a plugin that starts the language server (`tools/bastio-lsp`).

The grammars follow the syntax that stage0 accepts (`stage0/src/lexer.rs`, `stage0/src/parser.rs`). CI parses every `.bastio` file in this repository with the tree-sitter grammar and tokenizes it with the TextMate grammar, so a change to the syntax that the editors do not follow fails there.
