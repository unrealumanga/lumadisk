# LumaDisk

> **Find the space hogs. Spot duplicates. Clean up with confidence.**

[![Cross-platform build](https://github.com/unrealumanga/lumadisk/actions/workflows/build.yml/badge.svg)](https://github.com/unrealumanga/lumadisk/actions/workflows/build.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-51e5b1.svg)](LICENSE)
[![Made with Rust](https://img.shields.io/badge/made%20with-Rust-orange.svg)](https://www.rust-lang.org/)
[![Windows, macOS, Linux](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-6d7bea.svg)](#platform-support)

LumaDisk is a fast, private disk space analyzer for **Windows, macOS, and Linux**. Imagine a crypto-market heatmap, except every tile is one of your files:

- A **larger tile** means a larger file.
- A **color** represents the file type or age.
- A **click** shows what the file is and where it lives.

Everything happens locally. LumaDisk does not upload file names, paths, or content.

## Why LumaDisk?

Storage fills up quietly. Normal file lists make it hard to see what matters. LumaDisk turns thousands of files into one clear picture, so you can quickly decide what to keep, move, or send to Trash.

| Feature | What it means for you |
| --- | --- |
| Storage heatmap | See the biggest files immediately |
| Largest-files list | Review space hogs from largest to smallest |
| Exact duplicate finder | Find byte-for-byte copies and reclaim wasted space |
| File-type categories | Select all visible `.obj`, `.max`, `.zip`, or other files together |
| Smart filters | Narrow results by name, type, size, or modified date |
| Native file location | Reveal a file in Explorer, Finder, or your Linux file manager |
| Safe cleanup | Files go to Trash / Recycle Bin after confirmation |
| Local-first privacy | No account, cloud upload, telemetry, or background service |

## Quick start

1. Open LumaDisk.
2. Choose a folder, drive, external disk, or mounted network share.
3. Select **Scan folder**.
4. Click a tile, duplicate, or colored category header to review it.
5. Keep it, move it, reveal its location, or send it to Trash.

> [!IMPORTANT]
> A category cleanup uses your active filters. LumaDisk always shows the exact number of visible files and their combined size before asking for confirmation.

## Download

Download the [latest LumaDisk release](https://github.com/unrealumanga/lumadisk/releases/latest):

- Windows x64: portable `.zip`
- macOS Apple Silicon: `.app.zip`
- macOS Intel: `.app.zip`
- Linux x64: portable `.tar.gz`

Extract the archive and launch LumaDisk. Each download includes a matching SHA-256 checksum file. These early builds are not yet code-signed or notarized, so your operating system may show an unfamiliar-developer warning.

## Build from source

Install the current stable [Rust toolchain](https://rustup.rs/), then run:

```bash
git clone https://github.com/unrealumanga/lumadisk.git
cd lumadisk
cargo run --release
```

On Debian or Ubuntu, install the desktop libraries first:

```bash
sudo apt-get install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libgl1-mesa-dev
```

The optimized binary is written to `target/release/lumadisk` (`lumadisk.exe` on Windows).

## Platform support

| Platform | Architecture | CI build | Native integrations |
| --- | --- | --- | --- |
| Windows | x64 | Yes | Explorer and Recycle Bin |
| macOS | Apple Silicon | Yes | Finder and Trash |
| macOS | Intel x64 | Yes | Finder and Trash |
| Linux | x64 | Yes | File manager and freedesktop Trash |

The core is written in Rust. LumaDisk uses a native desktop window rather than bundling a browser or running a local web server.

## How duplicate detection stays fast

LumaDisk does not hash every file blindly. It first groups files by size, compares small samples from likely matches, and only then verifies candidates with a complete BLAKE3 hash. Equal-size files with different content are not reported as duplicates.

Deleting or moving one reviewed copy updates the remaining duplicate groups immediately. A rescan is not required.

## Privacy and safety

- Files and scan results stay on your computer.
- Symbolic links are never followed.
- Nothing is permanently deleted by LumaDisk.
- Bulk cleanup always requires confirmation.
- There is no updater, network service, command shell, process injection, or telemetry.

No developer can guarantee that every antivirus product will trust every newly built executable. Public releases should be code-signed on Windows, signed and notarized on macOS, and published with checksums on every platform.

## What is next?

- Saved include/exclude rules
- Incremental rescans for large drives and NAS locations
- Cross-format 3D model similarity for formats such as OBJ, FBX, GLB, USDZ, MAX, and Blend
- Optional local content search, OCR, and semantic indexing

Heavy AI features will remain optional so the normal disk scan stays quick and lightweight.

## Contributing

Bug reports and focused pull requests are welcome. Before submitting code, run:

```bash
cargo fmt --check
cargo test --locked
cargo clippy --all-targets -- -D warnings
```

## License

LumaDisk is available under the [MIT License](LICENSE).
