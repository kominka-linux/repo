# zig cc + musl build notes

## The memcpy symbol visibility problem

When building musl libc with `zig cc` as the toolchain, `memcpy`, `memset`,
`memmove`, and other string functions end up as `LOCAL HIDDEN` in the final
`libc.so` rather than `GLOBAL DEFAULT`, making them invisible to programs that
dynamically link against libc.so.

### What we know for certain

From `readelf -Ws libc.so`:
```
LOCAL  HIDDEN  memcpy    ← broken: not in .dynsym, programs can't resolve it
GLOBAL DEFAULT malloc    ← correct: in .dynsym
```

From linking the minimal case (just `memcpy.lo`):
```
zig ld.lld --shared --export-dynamic -o t.so memcpy.lo
→ GLOBAL DEFAULT memcpy ✓   (works fine in isolation)
```

So the problem only appears when more objects are linked together.

### Root cause confirmed (2026-04-13)

**The four sources that demote memcpy to LOCAL HIDDEN:**

1. **ubsan_rt** (zig's UBSan runtime): defines `memcpy`/`memset`/`memmove`
   as WEAK HIDDEN in a single monolithic ZCU object. zig cc adds ubsan_rt to
   every link **without `--as-needed`**, pulling in the whole archive. HIDDEN
   beats DEFAULT in ELF visibility merging → GLOBAL HIDDEN → demoted to LOCAL.

2. **compiler_rt** (LLVM's compiler runtime): same as ubsan_rt — defines these
   functions as WEAK HIDDEN. zig cc adds compiler_rt **with `--as-needed`**, but
   if any compiler_rt symbol is referenced (e.g., `__subtf3` for long double
   math), the whole monolithic .o is pulled in including WEAK HIDDEN memcpy.

3. **zig cc's default UBSan compile flags**: zig cc enables
   `-fsanitize=alignment,array-bounds,...` by default during compilation. This
   causes functions that call `memmove`/`memset` (like `bcopy.c`, `bzero.c`)
   to generate hidden memcpy references in their compiled objects. Adding these
   objects to the link triggers more compiler_rt/ubsan_rt pulls.

4. **zig ld.lld self-relocation crash**: using `zig ld.lld` directly (not via
   `zig cc`) avoids the runtime injection, but produces a libc.so whose ldso
   crashes when loading any dynamically-linked program. Strace shows the crash
   at a small unmapped address (e.g., `0xb7f90`) — consistent with the ldso
   computing `base = 0` during self-relocation instead of the actual load
   address. The exact mechanism differs from Alpine's zig cc / ld.bfd link.

### Approaches attempted and results

| Approach | Result |
|----------|--------|
| zig ld.lld directly | memcpy GLOBAL DEFAULT ✓, but ldso crashes on any dynamic binary ✗ |
| zig cc + drop ubsan_rt via compiler_rt stubs | ubsan_rt STILL added without --as-needed |
| zig cc + -fno-sanitize=all (link step only) | ubsan_rt still added (flag only disables instrumentation) |
| zig cc + -fno-sanitize=all in CFLAGS_AUTO (compile) | ubsan_rt still pulled in from zig cc's link step |
| zig cc + -Wl,--as-needed + -fno-sanitize=all | ubsan_rt linked but `.text.memcpy` still causes visibility issue |
| Float128 stubs in float128_shim.c | Prevents compiler_rt but not ubsan_rt |
| --dynamic-list, --version-script, --export-dynamic-symbol | Can't promote LOCAL to GLOBAL (ELF spec constraint) |

### Current workaround (rel=14 / mimalloc rel=1)

musl uses its built-in **mallocng** allocator. mimalloc is shipped as a
**separate shared library** (`libmimalloc.so.2`) and loaded via
`/etc/ld.so.preload`, which causes musl's dynamic linker to preload it before
any program starts. This gives all dynamically-linked programs mimalloc's
allocator system-wide.

### The zig ld.lld self-relocation crash (detail)

When `zig ld.lld` links libc.so, the resulting ldso crashes at startup for
any dynamically-linked program. Symptoms:

- Static binaries (no PT_INTERP) work fine
- Dynamic binaries (NEEDED: libc.so) crash immediately with SIGSEGV
- strace shows: `mprotect(addr, size, PROT_READ)` for RELRO, then SIGSEGV at
  a small address like `0xb7f90`

The crash address being small (link-time value, not base+offset) indicates
the ldso computed `base = 0` during self-relocation. All RELATIVE relocation
GOT entries end up pointing to their link-time addresses instead of runtime
addresses. Any function call through these corrupted GOT entries crashes.

Alpine's ldso (linked with gcc + ld.bfd) doesn't have this issue. The exact
difference between zig ld.lld-generated PLT/GOT layout and what musl's ldso
self-relocation code expects has not been isolated.

### What still needs to be solved

To properly fix musl + mimalloc integration (or any other allocator in libc):

1. **Prevent zig cc from adding ubsan_rt/compiler_rt with WEAK HIDDEN memcpy**:
   - `zig objcopy` at /usr/bin/objcopy (zig's wrapper) doesn't support
     `--localize-symbol` or `--strip-symbol`
   - GNU binutils `objcopy` (Alpine) is overridden by zig's wrapper script
   - `--rtlib=none` equivalent for zig cc not tested
   - May need to intercept the lld invocation and filter the archives

2. **Fix the zig ld.lld self-relocation crash**:
   - Might be related to the 4-LOAD-segment layout vs Alpine's 2-LOAD layout
   - Might be related to missing `-mllvm -float-abi=hard` or similar flags
   - Might be a zig 0.15.2 specific behavior that differs from ld.bfd
   - The `adrp+add → nop+adr` PLT relaxation done by zig ld.lld may be
     relevant (it produces a different instruction sequence than Alpine's ldso)

3. **Alternative**: build musl with a non-zig compiler (clang or GCC with lld)
   that doesn't inject WEAK HIDDEN memory functions. Chimera Linux uses patches
   to build musl with clang+lld and exports memcpy correctly.

## Key zig cc behaviors affecting system library builds

- Enables broad UBSan sanitizers by default in ALL compilation steps
  (`-fsanitize=alignment,array-bounds,bool,builtin,...`)
- Adds `ubsan_rt` WITHOUT `--as-needed` to every link (even with -nostdlib)
- Adds `compiler_rt` WITH `--as-needed` to every link
- Both ubsan_rt and compiler_rt define `memcpy`/`memset`/`memmove` as
  WEAK HIDDEN in their monolithic single-object archives
- `-fno-sanitize=all` disables sanitizer instrumentation but NOT runtime linking
- `zig ld.lld` (direct linker invocation) produces different PLT/GOT than
  `zig cc` (which adds `-mllvm -float-abi=hard`, `-m aarch64linux`, etc.)
- The `adrp+add` instruction pair for loading `_DYNAMIC` is optimized to
  `nop+adr` by zig ld.lld; this may affect how musl's self-relocation computes
  its load base

## Testing memcpy export

```sh
# Check if memcpy is exported from libc.so (should show T memcpy)
nm -D libc.so | grep -E " T memcpy$| T memset$| T memmove$"

# Check if it's present but LOCAL HIDDEN (broken state)
readelf -Ws libc.so | grep memcpy

# Test that dynamically-linked programs work
zig cc -target aarch64-linux-musl -dynamic hello.c -o hello
chroot /with-our-musl /hello
```
