# TODO

## Packages
- **strace**: Bootstrap build fails on aarch64 — Alpine gcc 15.2.0's ld rejects
  hidden libgcc symbols (`__floatunsitf`, `__muldc3`) referenced by the intermediate
  `ioctlsort1` helper binary. This is a pre-existing incompatibility between Alpine's
  gcc/binutils and the way strace's configure generates intermediate tools. Does not
  affect the strace build itself on our own toolchain — defer to when build-package.yml
  is fully working.
- **xz**: aarch64 build fails with lld 20.x strict alignment check
  (`R_AARCH64_LDST32_ABS_LO12_NC: 0x1047FAE is not aligned to 4 bytes`) in
  `src/xz/args.c`. The address is a static data item placed at a 2-byte-aligned
  address accessed via a 32-bit load instruction. GNU ld (bfd) doesn't enforce this;
  lld 20.x does. Needs an upstream fix or a targeted patch to `src/xz/args.c` to
  add `__attribute__((aligned(4)))` to the offending data, or a linker flag once one
  is available.
- **cargo**: Rust toolchain tarball (~200MB) exceeds the repo server upload limit.
  Build succeeds — artifact is available in GitHub Actions. Either raise the server
  limit or split the package.
- **libstdcxx**: Empty package directory — no PKGBUILD.ysh. Either add one or remove
  the directory.
- Source mirror: fetch and rehost all sources to avoid upstream dependency
  - could also trim fat to help lower disk requirements
    - boringssl-0.20260327.0 is a prime example; massive test/ dirs that can be removed
    - we can also standardize on bzip2
- builds should be done in a linux namespace container and in a maximally reproducible manner

## Infrastructure
- Server health monitoring / alerting

