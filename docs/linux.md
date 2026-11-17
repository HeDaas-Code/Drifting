# Linux

Linux builds target x86_64 and are distributed as an AppImage and a Debian
package. The AppImage is the portable option; the DEB package is intended for
Debian, Ubuntu and compatible distributions.

## Install

Download the matching files from the GitHub Release:

- `Drifting_<version>_linux_x86_64.AppImage`: make it executable and run it.
- `Drifting_<version>_linux_x86_64.deb`: install with `apt` or your package manager.

The AppImage and DEB package need a desktop session with GTK 3 and WebKitGTK
4.1. Linux secure storage uses the Secret Service implementation provided by
Libsecret. If no Secret Service provider is available, entering an API key or
signing in will fail clearly instead of writing credentials to a plain file.

Linux does not support HEIC, HEIF or AVIF decoding in the native image pipeline;
PNG, JPEG, GIF, WebP, BMP, ICO and TIFF are supported. PDF thumbnails use the
same renderer fallback as the other desktop targets.

## Build locally

Install Node.js 22.23.1 or newer, pnpm 10.17.1, Rust 1.96 or newer, and the
Tauri Linux prerequisites. On Ubuntu 24.04:

```bash
sudo apt-get install build-essential curl file libgtk-3-dev libsecret-1-dev \
  libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf \
  pkg-config
```

Then build the local-only client:

```bash
pnpm install --frozen-lockfile
pnpm tauri:build -- --bundles appimage,deb
```

The artifacts are written below
`src-tauri/target/release/bundle/{appimage,deb}`. For development, use
`pnpm dev` after installing the same prerequisites.

## Data locations

Linux stores the local library under
`${XDG_DATA_HOME:-~/.local/share}/cc.drifting.client/`. Development builds use
the checkout's ignored `.local-data` directory through the existing worktree
launcher, so they do not share data with an installed release.
