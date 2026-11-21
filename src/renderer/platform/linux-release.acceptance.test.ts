import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

// @ts-expect-error The executable Node script intentionally lives outside the renderer TS graph.
import { mergeUpdateManifest } from '../../../scripts/merge-update-manifest.mjs';

const source = (path: string) => readFileSync(path, 'utf8');

describe('Linux desktop release', () => {
  it('uses the native title bar and produces the documented package formats', () => {
    const config = JSON.parse(source('src-tauri/tauri.linux.conf.json'));
    expect(config.app.windows[0]).toMatchObject({
      titleBarStyle: 'Visible',
      hiddenTitle: false,
      decorations: true,
    });
    expect(config.bundle.targets).toEqual(['appimage', 'deb']);
    expect(source('docs/linux.md')).toContain('libwebkit2gtk-4.1-dev');
    expect(source('docs/linux.md')).toContain('Drifting_<version>_linux_x86_64.AppImage');
  });

  it('builds and publishes Linux x86_64 artifacts with the signed updater entry', () => {
    const release = source('.github/workflows/release.yml');
    expect(release).toMatch(/^ {2}build-linux:\n {4}name: build \(linux x86_64\)$/mu);
    expect(release).toContain('--target x86_64-unknown-linux-gnu --bundles appimage,deb');
    expect(release).toContain('linux-x86_64');
    expect(release).toContain('Drifting_${VERSION}_linux_x86_64.AppImage.tar.gz');

    const manifest = {
      version: '0.1.0-alpha.11',
      notes: 'Notes',
      pub_date: '2026-11-17T00:00:00.000Z',
      platforms: { 'darwin-aarch64': { signature: 'mac', url: 'https://example.test/mac.tar.gz' } },
    };
    const merged = mergeUpdateManifest(manifest, {
      platform: 'linux-x86_64',
      url: 'https://example.test/linux.AppImage.tar.gz',
      signature: 'linux-signature\n',
      version: manifest.version,
    });
    expect(merged.platforms['linux-x86_64']).toEqual({
      signature: 'linux-signature',
      url: 'https://example.test/linux.AppImage.tar.gz',
    });
    expect(merged.platforms['darwin-aarch64']).toEqual(manifest.platforms['darwin-aarch64']);
  });

  it('runs native Linux validation in the preview workflow', () => {
    const workflow = source('.github/workflows/linux.yml');
    expect(workflow).toContain('windows-build:');
    expect(workflow).toContain('runs-on: windows-latest');
    expect(workflow).toContain('--target x86_64-pc-windows-msvc --bundles nsis');
    expect(workflow).toContain('Install and smoke test Drifting');
    expect(workflow).toContain('runs-on: ubuntu-24.04');
    expect(workflow).toContain('cargo test --manifest-path src-tauri/Cargo.toml --locked --lib');
    expect(workflow).toContain('pnpm tauri:build -- --bundles appimage,deb');
    expect(workflow).toContain('xvfb-run');
  });
});
