import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';

const errors = [];
const ci = readFileSync('.github/workflows/ci.yml', 'utf8');
const release = readFileSync('.github/workflows/release.yml', 'utf8');
const windows = readFileSync('.github/workflows/windows.yml', 'utf8');
const targetPolicy = readFileSync('.github/workflows/pr-target.yml', 'utf8');
const linux = readFileSync('.github/workflows/linux.yml', 'utf8');

function requireMatch(source, pattern, message) {
  if (!pattern.test(source)) errors.push(message);
}

const push = /^  push:\n((?: {4}[^\n]*\n)*)/mu.exec(ci)?.[1] ?? '';
const pushBranches = /^ {4}branches: \[([^\]]+)\]$/mu.exec(push)?.[1]
  ?.split(',').map((branch) => branch.trim().replace(/^['"]|['"]$/gu, '')) ?? [];
for (const branch of ['main', 'next']) {
  if (!pushBranches.includes(branch)) errors.push(`ordinary CI must run for ${branch} pushes`);
}
requireMatch(ci, /^\s{2}pull_request:\s*$/mu, 'ordinary CI must run for pull requests');
if (/^\s{4}tags:/mu.test(ci)) {
  errors.push('ordinary CI must not duplicate the release workflow on tags');
}

for (const job of ['client-checks', 'client-tests', 'client-renderer', 'client', 'native']) {
  requireMatch(ci, new RegExp(`^  ${job}:\\s*$`, 'mu'), `CI is missing the ${job} job`);
}
requireMatch(ci, /^\s{4}name: client\s*$/mu, 'CI must preserve the required client context');
requireMatch(
  ci,
  /^\s{4}needs: \[client-checks, client-tests, client-renderer\]\s*$/mu,
  'client must aggregate checks, test shards and renderer contracts',
);
requireMatch(ci, /pnpm exec vitest run --shard="\$TEST_SHARD"/u, 'CI must run sharded Vitest');
requireMatch(ci, /pnpm perf:renderer --ci --output=\.local-data\/renderer-performance\/ci\.json/u, 'CI must generate and validate current deterministic renderer evidence');
requireMatch(ci, /DRIFTING_PERF_CHROME: \/usr\/bin\/google-chrome/u, 'renderer CI must select the installed Chrome executable explicitly');
requireMatch(ci, /pnpm perf:renderer:lifecycle --output=\.local-data\/renderer-performance\/lifecycle\.json/u, 'CI must check renderer resource release');
requireMatch(ci, /test "\$RENDERER_RESULT" = success/u, 'client must require renderer success');
requireMatch(ci, /RENDERER_RESULT: \$\{\{ needs\.client-renderer\.result \}\}/u, 'renderer gate must read the renderer job result');
const rustAction = 'dtolnay/rust-toolchain@02cb101ec7c40f2c49e1d9714d64511d8e1b74de';
for (const [name, workflow, expectedCount] of [
  ['ordinary CI', ci, 2],
  // verify, the macOS build, the Windows build and the Linux build
  ['release', release, 4],
]) {
  if (workflow.split(rustAction).length - 1 !== expectedCount)
    errors.push(`${name} must use the pinned Rust action that accepts the toolchain input`);
  if (workflow.split('toolchain: 1.96.0').length - 1 !== expectedCount)
    errors.push(`${name} must select Rust 1.96.0 for every Rust job`);
}

for (const command of [
  'pnpm ci:contract:check',
  'pnpm openai:catalog:check',
  'pnpm public:check',
  'pnpm core:test',
  'pnpm security:dependencies',
  'pnpm security:rust',
  'pnpm lint',
  'pnpm typecheck',
  'pnpm agent:capabilities:check',
  'pnpm agent:conversation-sync:check',
  'pnpm exec vite build --config vite.renderer.config.ts',
]) {
  if (!ci.includes(command)) errors.push(`client checks are missing: ${command}`);
}
requireMatch(release, /^  workflow_call:$/mu, 'release must be invoked by the CI workflow');
requireMatch(release, /github.event_name == 'push' && github.ref == 'refs\/heads\/main'/u, 'release must only run on a main push');
requireMatch(ci, /needs: \[client, native, linux-tests, windows-tests, next-tests, codex-compatibility\]/u, 'cloud tests must wait for all test workflows');
requireMatch(ci, /needs: \[cloud-tests, linux-preview, windows-preview, next-build, macos-preview\]/u, 'cloud build must wait for every preview platform');
requireMatch(targetPolicy, /"\$HEAD_BRANCH" == next && "\$HEAD_REPO" == "\$GITHUB_REPOSITORY"/u, 'only this repository’s next branch can promote to main');
for (const branch of ['main', 'next']) {
  const ruleset = JSON.parse(readFileSync(`.github/rulesets/${branch}.json`, 'utf8'));
  const contexts = ruleset.rules.find(({ type }) => type === 'required_status_checks')
    ?.parameters.required_status_checks.map(({ context }) => context) ?? [];
  for (const check of ['PR target', 'Cloud tests', 'Cloud build']) {
    if (!contexts.includes(check)) errors.push(`${branch} ruleset must require ${check}`);
  }
}
if (!release.includes('pnpm ci:contract:check')) {
  errors.push('release validation must enforce the CI contract');
}
requireMatch(linux, /^name: Desktop Builds\s*$/mu, 'desktop build workflow must be present');
requireMatch(linux, /runs-on: ubuntu-24\.04/u, 'Linux workflow must use Ubuntu 24.04');
requireMatch(linux, /cargo test --manifest-path src-tauri\/Cargo\.toml --locked --lib/u, 'Linux workflow must run native Rust tests');
requireMatch(linux, /pnpm tauri:build -- --bundles appimage,deb/u, 'Linux workflow must build AppImage and DEB artifacts');
requireMatch(windows, /^  installer:\s*$/mu, 'desktop workflow must build Windows');
requireMatch(windows, /runner: windows-latest/u, 'Windows build must use a Windows runner');
requireMatch(windows, /--bundles nsis/u, 'Windows build must produce an NSIS installer');

const testFiles = execFileSync('git', [
  'ls-files',
  '--cached',
  '--others',
  '--exclude-standard',
  '--',
  '*.test.ts',
  '*.test.tsx',
  '*.test.js',
  '*.test.mjs',
  '*.spec.ts',
  '*.spec.tsx',
])
  .toString('utf8')
  .split('\n')
  .filter((path) => Boolean(path) && existsSync(path));
const mutableVersionAssertion = /\.toContain\(\s*['"`]version:\s*\d+,?['"`]\s*\)/u;
for (const path of testFiles) {
  const source = readFileSync(path, 'utf8');
  if (mutableVersionAssertion.test(source)) {
    errors.push(
      `${path} locks an acceptance test to a mutable current store version; test migration behavior instead`,
    );
  }
}

if (errors.length > 0) {
  console.error(`CI contract check failed with ${errors.length} issue(s):`);
  for (const error of errors) console.error(`- ${error}`);
  process.exit(1);
}

console.log(`CI contract check passed for ${testFiles.length} tracked test files.`);
