// Runs the Bastio compiler on save and shows its error in the editor.
//
// This is not a language server: it shells out to bastioc0 (by default with
// `--check`, which stops after the proofs) and turns its diagnostic,
// `file:line:col: error[CODE]: message`, into a VS Code diagnostic. stage0
// stops at the first error, so there is at most one at a time.

'use strict';

const vscode = require('vscode');
const cp = require('child_process');
const path = require('path');

const DIAG_RE = /^(.+?):(\d+):(\d+): error\[([A-Z0-9-]+)\]: (.*)$/;

let collection;
let warnedMissing = false;
// One run at a time per checked file; a save during a run restarts it.
const running = new Map();

function activate(context) {
  collection = vscode.languages.createDiagnosticCollection('bastio');
  context.subscriptions.push(collection);
  context.subscriptions.push(
    vscode.workspace.onDidSaveTextDocument((doc) => {
      if (doc.languageId === 'bastio' && config().get('checkOnSave')) check(doc);
    }),
    vscode.workspace.onDidCloseTextDocument((doc) => {
      if (doc.languageId === 'bastio' && !config().get('compiler.entry')) collection.delete(doc.uri);
    }),
    vscode.commands.registerCommand('bastio.check', () => {
      const ed = vscode.window.activeTextEditor;
      if (ed && ed.document.languageId === 'bastio') check(ed.document);
    }),
  );
}

function config() {
  return vscode.workspace.getConfiguration('bastio');
}

function check(doc) {
  const folder = vscode.workspace.getWorkspaceFolder(doc.uri);
  const cwd = folder ? folder.uri.fsPath : path.dirname(doc.uri.fsPath);
  const resolve = (p) => (path.isAbsolute(p) ? p : path.join(cwd, p));

  let compiler = config().get('compiler.path') || 'bastioc0';
  if (compiler.includes('/') || compiler.includes('\\')) compiler = resolve(compiler);
  const entry = config().get('compiler.entry');
  const target = entry ? resolve(entry) : doc.uri.fsPath;
  const args = [...(config().get('compiler.args') || []), target];

  const prev = running.get(target);
  if (prev) prev.kill();
  const child = cp.execFile(compiler, args, { cwd, timeout: 120000, maxBuffer: 4 << 20 }, (err, stdout, stderr) => {
    if (running.get(target) === child) running.delete(target);
    if (err && err.killed) return; // replaced by a newer run, or timed out
    if (err && err.code === 'ENOENT') {
      if (!warnedMissing) {
        warnedMissing = true;
        vscode.window.showWarningMessage(
          `Bastio: cannot run the compiler \`${compiler}\`. Set bastio.compiler.path to your bastioc0.`,
        );
      }
      return;
    }
    collection.clear();
    const byFile = new Map();
    for (const d of parse(stderr + stdout, resolve)) {
      if (!byFile.has(d.file)) byFile.set(d.file, []);
      byFile.get(d.file).push(d.diag);
    }
    for (const [file, diags] of byFile) collection.set(vscode.Uri.file(file), diags);
  });
  running.set(target, child);
}

// Turns the compiler's output into diagnostics. A line `  = note` after an
// error is appended to its message.
function parse(text, resolve) {
  const out = [];
  let last = null;
  for (const line of text.split('\n')) {
    const m = DIAG_RE.exec(line);
    if (m) {
      const [, file, l, c, code, msg] = m;
      // A file without `main` is a library: checking it alone says only that.
      if (code === 'E-TYPE' && /^no `main` function/.test(msg)) {
        last = null;
        continue;
      }
      const ln = Math.max(0, Number(l) - 1);
      const col = Math.max(0, Number(c) - 1);
      const diag = new vscode.Diagnostic(
        new vscode.Range(ln, col, ln, col + 1),
        msg,
        vscode.DiagnosticSeverity.Error,
      );
      diag.code = code;
      diag.source = 'bastioc0';
      last = diag;
      out.push({ file: resolve(file), diag });
    } else if (last && line.startsWith('  = ')) {
      last.message += '\n' + line.slice(4);
    }
  }
  return out;
}

function deactivate() {
  for (const child of running.values()) child.kill();
}

module.exports = { activate, deactivate, parse, DIAG_RE };
