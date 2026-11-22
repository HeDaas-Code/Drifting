import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';

const [before, head, notesPath, commitsPath, output] = process.argv.slice(2);
const isSha = (value) => /^[a-f0-9]{40}$/u.test(value ?? '');
if (!isSha(before) || !isSha(head) || !notesPath || !commitsPath || !output) {
  throw new Error('Expected before SHA, head SHA, notes path, commits path and output path');
}

const range = /^0+$/u.test(before) ? head : `${before}..${head}`;
const commits = execFileSync('git', ['log', '--format=fuller', '--date=iso-strict', range], {
  encoding: 'utf8',
  maxBuffer: 32 * 1024 * 1024,
});
writeFileSync(commitsPath, commits);

const notes = readFileSync(notesPath, 'utf8').trim();
let fenceLength = 3;
for (const [run] of commits.matchAll(/`+/gu)) fenceLength = Math.max(fenceLength, run.length + 1);
const fence = '`'.repeat(fenceLength);
const withCommits = `${notes}\n\n## 本次主线更新的全部提交\n\n${fence}text\n${commits}${fence}\n`;
const limit = 120_000;
writeFileSync(output, Buffer.byteLength(withCommits) < limit
  ? withCommits
  : `${notes.slice(0, 20_000)}\n\n## 本次主线更新的全部提交\n\n完整提交记录见随附的 \`COMMITS.txt\`。\n`);
