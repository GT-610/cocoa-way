<img align="right" width="25%" src="https://github.com/Smithay/smithay/assets/20758186/7a84ab10-e229-4823-bad8-9c647546407b">

# 🌐 Smithay Universal (Fork)

> **Research Artifact**: This is a modified fork of [Smithay](https://github.com/Smithay/smithay) designed for cross-platform Wayland research.
> It enables **native Wayland compositors on macOS and Windows** without Linux kernel dependency, VMs, or X11 translation.

## 🚀 Key Improvements & Modifications

### 1. Universal Winit Backend
We have significantly overhauled the `backend::winit` module to serve as a production-grade backend rather than just a debug tool.
- **MacOS (Cocoa/CGL)**: Implemented native CGL context management, `IOSurface` mapping, and Retina scaling support.
- **Windows (WGL/EGL)**: (In Progress) Implementation of WGL context creation via `glutin`.
- **Latency Optimization**: Custom rendering loop integration to bypass strict OS windowing limitations.

### 2. Event Loop Fusion
- **CFRunLoop Integration**: Implements a `Calloop` Event Source that hooks directly into macOS `CFRunLoop`, allowing Wayland clients to be dispatched purely within the native macOS application loop.
- **Input Bridging**: Low-latency translation of `Winit` events to Wayland input events (keyboard, mouse, touchpad gestures).
### 3. Experimental: "Turbo-Charged" SIMD Acceleration
- **Zero-Cost Mapping**: Implements the "Zero-Cost Protocol Virtualization" architecture described in our IEEE TC paper draft.
- **NEON/AVX2 Swizzling**: Includes a hand-tuned [SIMD module](src/utils/simd_utils.rs) for `wl_shm` format conversion, bridging the performance gap on non-Linux platforms lacking `dmabuf`.
- **Validation**: Pass `swizzle_correctness` check via `cargo test` (or standalone `turbo_test.rs`).
- **📄 Paper**: Full manuscript and benchmarks at [../paper/](../paper/)

### 4. Build System
- Decoupled `gbm` and `libinput` dependencies to allow compilation on non-Linux targets.
- Added macOS-specific feature flags and linking arguments.

---

# Smithay (Original README)

# Smithay

[![Crates.io](https://img.shields.io/crates/v/smithay.svg)](https://crates.io/crates/smithay)
[![docs.rs](https://docs.rs/smithay/badge.svg)](https://docs.rs/smithay)
[![Build Status](https://github.com/Smithay/smithay/workflows/Continuous%20Integration/badge.svg)](https://github.com/Smithay/smithay/actions)
[![Join the chat on matrix at #smithay:matrix.org](https://img.shields.io/badge/%5Bm%5D-%23smithay%3Amatrix.org-blue.svg)](https://matrix.to/#/#smithay:matrix.org)
![Join the chat via bridge on #smithay on libera.chat](https://img.shields.io/badge/IRC-%23Smithay-blue.svg)

A smithy for rusty wayland compositors

## Goals

Smithay aims to provide building blocks to create wayland compositors in Rust. While not
being a full-blown compositor, it'll provide objects and interfaces implementing common
functionalities that pretty much any compositor will need, in a generic fashion.

It supports the [core Wayland protocols](https://gitlab.freedesktop.org/wayland/wayland), the official [protocol extensions](https://gitlab.freedesktop.org/wayland/wayland-protocols), and *some* external extensions, such as those made by and for [wlroots](https://gitlab.freedesktop.org/wlroots/wlr-protocols) and [KDE](https://invent.kde.org/libraries/plasma-wayland-protocols)
<!-- https://github.com/Smithay/smithay/pull/779#discussion_r993640470 https://github.com/Smithay/smithay/issues/778 -->

Also:

- **Documented:** Smithay strives to maintain a clear and detailed documentation of its API and its
  functionalities. Compiled documentations are available on [docs.rs](https://docs.rs/smithay) for released
  versions, and [here](https://smithay.github.io/smithay) for the master branch.
- **Safety:** Smithay will target to be safe to use, because Rust.
- **Modularity:** Smithay is not a framework, and will not be constraining. If there is a
  part you don't want to use, you should not be forced to use it.
- **High-level:** You should be able to not have to worry about gory low-level stuff (but 
  Smithay won't stop you if you really want to dive into it).


## Anvil

Smithay as a compositor library has its own sample compositor: anvil.

To get informations about it and how you can run it visit [anvil README](https://github.com/Smithay/smithay/blob/master/anvil/README.md)

## Other compositors that use Smithay

- [Cosmic](https://github.com/pop-os/cosmic-epoch): Next generation Cosmic desktop environment
- [Catacomb](https://github.com/catacombing/catacomb): A Wayland Mobile Compositor
- [MagmaWM](https://github.com/MagmaWM/MagmaWM): A versatile and customizable Wayland Compositor
- [Niri](https://github.com/YaLTeR/niri): A scrollable-tiling Wayland compositor
- [Strata](https://github.com/StrataWM/strata): A cutting-edge, robust and sleek Wayland compositor
- [Pinnacle](https://github.com/Ottatop/pinnacle): A WIP Wayland compositor, inspired by AwesomeWM 
- [Sudbury](https://gitlab.freedesktop.org/bwidawsk/sudbury): Compositor designed for ChromeOS
- [wprs](https://github.com/wayland-transpositor/wprs): Like [xpra](https://en.wikipedia.org/wiki/Xpra), but for Wayland, and written in
Rust.
- [Local Desktop](https://github.com/localdesktop/localdesktop): An Android app for running GUI Linux via PRoot and Wayland.

## System Dependencies

(This list can depend on features you enable)

- `libwayland`
- `libxkbcommon`
- `libudev`
- `libinput`
- `libgbm`
- [`libseat`](https://git.sr.ht/~kennylevinsen/seatd)
- `xwayland`

## Contact us

If you have questions or want to discuss the project with us, our main chatroom is on Matrix: [`#smithay:matrix.org`](https://matrix.to/#/#smithay:matrix.org).

## License

This project (Smithay-Universal) is licensed under the **GNU General Public License v3.0**.

The original [Smithay](https://github.com/Smithay/smithay) library remains under the MIT license. Modifications in this fork are licensed under GPLv3.

Permissions of this strong copyleft license are conditioned on making available complete source code of licensed works and modifications, which include larger works using a licensed work, under the same license. Copyright and license notices must be preserved. Contributors provide an express grant of patent rights.

See the [LICENSE](LICENSE) file for details.
