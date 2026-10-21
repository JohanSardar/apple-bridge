import ts from 'typescript';
import { readdir, readFile } from 'node:fs/promises';
const failures = [];
async function check(path) {
  for (const entry of await readdir(path, { withFileTypes: true })) {
    if (['node_modules', 'dist', 'test-results', 'playwright-report'].includes(entry.name)) continue;
    const file = path + '/' + entry.name;
    if (entry.isDirectory()) { await check(file); continue; }
    if (!/\.(ts|tsx|css|mjs|js)$/.test(file)) continue;
    const source = await readFile(file, 'utf8');
    if (file.endsWith('.css')) {
      let quote = '', escaped = false;
      for (let i = 0; i < source.length; i++) {
        const c = source[i];
        if (escaped) { escaped = false; continue; }
        if (c === '\\') { escaped = true; continue; }
        if (quote) { if (c === quote) quote = ''; continue; }
        if (c === '"' || c === "'") { quote = c; continue; }
        if (c === '/' && source[i + 1] === '*') { failures.push(file + ': comment'); break; }
      }
      continue;
    }
    const tree = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, file.endsWith('.tsx') ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
    const scanner = ts.createScanner(ts.ScriptTarget.Latest, false, ts.LanguageVariant.Standard, source);
    const spans = [];
    function visit(node) {
      if (ts.isStringLiteralLike(node) || ts.isRegularExpressionLiteral(node) || ts.isJsxText(node)) spans.push([node.getStart(tree), node.end]);
      if (ts.isTemplateExpression(node)) {
        spans.push([node.head.getStart(tree), node.head.end]);
        for (const span of node.templateSpans) spans.push([span.literal.getStart(tree), span.literal.end]);
      }
      ts.forEachChild(node, visit);
    }
    visit(tree);
    spans.sort((a,b) => a[0]-b[0]);
    let span = 0;
    while (scanner.getTextPos() < source.length) {
      const pos = scanner.getTextPos();
      while (span < spans.length && spans[span][1] <= pos) span++;
      if (span < spans.length && pos >= spans[span][0] && pos < spans[span][1]) { scanner.setTextPos(spans[span][1]); continue; }
      const token = scanner.scan();
      if ([ts.SyntaxKind.SingleLineCommentTrivia, ts.SyntaxKind.MultiLineCommentTrivia].includes(token)) failures.push(file + ':' + (tree.getLineAndCharacterOfPosition(scanner.getTokenPos()).line + 1) + ': comment');
      if (token === ts.SyntaxKind.EndOfFileToken) break;
    }
  }
}
await check('.');
if (failures.length) { process.stderr.write(failures.join('\n') + '\n'); process.exitCode = 1; } else process.stdout.write('web comments: ok\n');
