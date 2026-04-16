# TODO

## Packages
- **strace**: Bootstrap build fails on aarch64 — Alpine gcc 15.2.0's ld rejects
  hidden libgcc symbols (`__floatunsitf`, `__muldc3`) referenced by the intermediate
  `ioctlsort1` helper binary. This is a pre-existing incompatibility between Alpine's
  gcc/binutils and the way strace's configure generates intermediate tools. Does not
  affect the strace build itself on our own toolchain — defer to when build-package.yml
  is fully working.

## Known Build Environment Limitations

- **x86_64 build-package.yml (self-hosted)**: Our boringssl shared library crashes with
  SIGSEGV during SSL init when built with `zig cc -target x86_64-linux-musl`. Root cause
  not isolated after extensive investigation (OPENSSL_NO_ASM, -fno-sanitize=all,
  -fno-exceptions, -fno-rtti, static/dynamic, etc. all attempted). The crash manifests
  in `curl --version` crashing in build-essential, preventing source tarball downloads.
  **Workaround**: use `bootstrap-build-package.yml` for all amd64 package builds — it
  runs in Alpine's Docker environment and uses Alpine's working curl.
  arm64 `build-package.yml` (self-hosted) is fully functional.
  See docs/ZIG-CC.md for the full investigation.

## Infrastructure
- Server health monitoring / alerting

## pm.ysh / YSH
- **SIGCHLD accounting**: YSH limitation — the process group SIGCHLD accounting in oils
  fires for all descendant exits, not just direct children. Nothing pm can do about it
  without a patch to oils itself.

