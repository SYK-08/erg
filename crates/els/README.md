# els (erg-language-server)

ELS is a language server for the [Erg](https://github.com/erg-lang/erg) programming language.

## Features

- [x] Syntax highlighting (by [vscode-erg](https://github.com/erg-lang/vscode-erg))
- [x] Code completion
  - [x] Variable completion
  - [x] Method/attribute completion
  - [x] Smart completion (considering type, parameter names, etc.)
  - [x] Auto-import
- [x] Diagnostics
- [x] Hover
- [x] Go to definition
- [ ] Go to implementation
- [x] Find references
- [x] Renaming
- [x] Inlay hint
- [x] Semantic tokens
- [x] Code actions
  - [x] eliminate unused variables
  - [x] change variable case
- [x] Code lens
  - [x] show trait implementations

## Installation

```console
cargo install erg --features els
```
