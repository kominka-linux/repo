# zig cc + musl build notes

## The memcpy symbol visibility problem

When building musl libc with `zig cc -target aarch64-linux-musl`, the standard
C library functions `memcpy`, `memset`, `memmove`, etc. end up as LOCAL symbols
(`t` in nm) in the final `libc.so` rather than GLOBAL (`T`), making them
invisible to programs that link against libc.so.

### Root cause analysis

musl's build system uses three mechanisms together to control symbol exports:

1. **`-fvisibility=hidden`** (CFLAGS_AUTO) — hides all C function symbols
2. **`--dynamic-list=../dynamic.list`** (LDFLAGS_AUTO) — explicitly exports
   the public API (malloc, environ, etc.)
3. **`--gc-sections`** (LDFLAGS_AUTO) — removes unreferenced sections

With zig cc / lld as the toolchain:
- `-fvisibility=hidden` is **not added** by zig cc's musl configure
- `--gc-sections` **is** supported and added
- `--dynamic-list` is **not supported** by lld → configure skips it

The aarch64-specific assembly file `src/string/aarch64/memcpy.S` defines
`memcpy` with `.global memcpy` (default visibility). However, zig cc (clang)
injects LOCAL `__memcpy_chk` wrappers and LOCAL `memcpy` implementations via
`_FORTIFY_SOURCE` into each compiled C file. These LOCAL definitions shadow
the GLOBAL assembly memcpy in the final shared library link.

Additionally: `zig cc` as a link driver includes `libubsan_rt.a` (without
`--as-needed`) and `libcompiler_rt.a` (with `--as-needed`) in the link step,
even with `-nostdlib`. These inject additional LOCAL memcpy implementations.

### Fix applied (musl PKGBUILD.ysh)

After `configure` runs and generates `config.mak`, patch it to:

1. Remove `--gc-sections` from `LDFLAGS_AUTO` — prevents dead-section removal
2. Remove `-fvisibility=hidden` from `CFLAGS_AUTO` — not added by zig cc but
   patched for safety
3. Add `-U_FORTIFY_SOURCE` to `CFLAGS_AUTO` — disables fortify injection
4. Patch `Makefile` to add `--export-dynamic` to the libc.so link — forces
   all global symbols into .dynsym

### Root cause of LOCAL memcpy (confirmed rel=10 investigation)

Testing with `zig ld.lld` directly:
- Linking `memcpy.lo` alone → GLOBAL `T memcpy` ✓
- Linking `memcpy.lo + strlen.lo` → LOCAL `t memcpy`, plus `__memcpy_chk` appears

The `__memcpy_chk` comes from `libubsan_rt.a`. zig cc includes this archive
**without `--as-needed`** by default, causing all ubsan symbols to be pulled
in — including `__memcpy_chk` and a LOCAL memcpy implementation that shadows
the GLOBAL assembly version.

`libubsan_rt.a` is the UBSan sanitizer runtime, intended for user code compiled
with `-fsanitize=undefined`. It should never be linked into libc itself.

### Fix applied (rel=11)

When patching the Makefile to use `zig ld.lld` directly:

- **Drop `libubsan_rt.a` entirely** — libc doesn't need the ubsan runtime
- Keep `libcompiler_rt.a` with `--as-needed` — needed for compiler support
  functions like `__subtf3` (128-bit float subtract used by some C99 math)

Link command: `zig ld.lld --shared ... --as-needed /path/to/libcompiler_rt.a`

## Key zig cc behaviors affecting system library builds

- Injects `libubsan_rt.a` without `--as-needed` in all link steps
- Injects `libcompiler_rt.a` with `--as-needed`  
- Enables `_FORTIFY_SOURCE` by default, injecting LOCAL `__memcpy_chk`
  wrappers into each compiled object
- Does NOT support musl's `--dynamic-list` via lld for controlling exports
- lld with `-Wl,--gc-sections` removes unreferenced functions unless protected
  by `--dynamic-list` or `--export-dynamic`

## Testing memcpy export

```sh
# Check if memcpy is exported from libc.so
nm -D libc.so | grep -E " T memcpy$| T memset$| T memmove$"

# Check if it's present but LOCAL (problem state)
nm libc.so | grep -E " t memcpy$| t memset$"

# Test that linked programs can find memcpy at runtime
zig cc -nostdinc -nostdlib -isystem /path/to/musl/include \
  -o test test.c crt1.o crti.o crtn.o libc.so \
  -dynamic-linker /usr/lib/ld-musl-aarch64.so.1
/usr/lib/ld-musl-aarch64.so.1 ./test
```
