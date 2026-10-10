// Tokenizes every .bastio file in the repository with the TextMate grammar
// (the same engine VS Code uses) and fails on an `invalid` token, then checks
// the scopes of a few lines that are easy to get wrong.
//
// usage: (cd editors/vscode && npm ci && npm test)

'use strict';

const fs = require('fs');
const path = require('path');
const vt = require('vscode-textmate');
const onig = require('vscode-oniguruma');

const repo = path.resolve(__dirname, '../../..');
const grammarPath = path.join(__dirname, '../syntaxes/bastio.tmLanguage.json');

function bastioFiles(dir, out = []) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) {
      if (!['.git', 'node_modules', 'target', 'editors', 'lexcases'].includes(e.name)) bastioFiles(p, out);
    } else if (e.name.endsWith('.bastio')) {
      out.push(p);
    }
  }
  return out;
}

function tokens(grammar, text) {
  let stack = vt.INITIAL;
  const out = [];
  for (const line of text.split('\n')) {
    const r = grammar.tokenizeLine(line, stack);
    stack = r.ruleStack;
    for (const t of r.tokens) out.push({ text: line.slice(t.startIndex, t.endIndex), scope: t.scopes[t.scopes.length - 1] });
  }
  return out;
}

// [source line, token text, expected innermost scope]
const EXPECT = [
  ['fn main() -> u64 {', 'main', 'entity.name.function.bastio'],
  ['fn main() -> u64 {', 'u64', 'support.type.builtin.bastio'],
  ['spec fn sq(x: u64) -> u64 { x * x }', 'sq', 'entity.name.function.bastio'],
  ['  requires forall i: u64 :: i < n', 'forall', 'keyword.other.proof.bastio'],
  // The body of a forall is not an enum variant.
  ['  requires forall i: u64 :: i < n', ' i ', 'source.bastio'],
  ['let o = Opt[u64]::Some(5);', 'Opt', 'entity.name.type.bastio'],
  ['let o = Opt[u64]::Some(5);', 'Some', 'entity.name.tag.variant.bastio'],
  ['let v = vec_new[Box[u64]]();', 'vec_new', 'entity.name.function.call.bastio'],
  ['const PAGE: u64 = 0x1000;', 'PAGE', 'variable.other.constant.bastio'],
  ['const PAGE: u64 = 0x1000;', '0x1000', 'constant.numeric.hex.bastio'],
  ['let s = "a\\n";', '\\n', 'constant.character.escape.bastio'],
  ['ensures result == old(x) ==> y // done', 'result', 'variable.language.result.bastio'],
  ['ensures result == old(x) ==> y // done', '==>', 'keyword.operator.logical.implies.bastio'],
  ['ensures result == old(x) ==> y // done', '// done', 'comment.line.double-slash.bastio'],
];

(async () => {
  const wasm = fs.readFileSync(require.resolve('vscode-oniguruma/release/onig.wasm'));
  await onig.loadWASM(wasm.buffer.slice(wasm.byteOffset, wasm.byteOffset + wasm.byteLength));
  const registry = new vt.Registry({
    onigLib: Promise.resolve({
      createOnigScanner: (s) => new onig.OnigScanner(s),
      createOnigString: (s) => new onig.OnigString(s),
    }),
    loadGrammar: async () => vt.parseRawGrammar(fs.readFileSync(grammarPath, 'utf8'), grammarPath),
  });
  const grammar = await registry.loadGrammar('source.bastio');
  let failed = 0;

  const files = bastioFiles(repo);
  for (const f of files) {
    const bad = tokens(grammar, fs.readFileSync(f, 'utf8')).filter((t) => t.scope.startsWith('invalid'));
    for (const t of bad) {
      console.log(`${path.relative(repo, f)}: invalid token ${JSON.stringify(t.text)}`);
      failed++;
    }
  }

  for (const [line, text, scope] of EXPECT) {
    const t = tokens(grammar, line).find((t) => t.text === text);
    const got = t ? t.scope : '(no such token)';
    if (got !== scope) {
      console.log(`${JSON.stringify(line)}: ${JSON.stringify(text)} is ${got}, expected ${scope}`);
      failed++;
    }
  }

  console.log(`tokenize_all: ${files.length} files, ${EXPECT.length} scope checks, ${failed} failures`);
  process.exitCode = failed ? 1 : 0;
})();
