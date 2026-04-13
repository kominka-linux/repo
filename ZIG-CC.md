# zig cc + musl build notes

## The memcpy symbol visibility problem

When building musl libc with `zig cc` as the toolchain, `memcpy`, `memset`,
`memmove`, and other string functions end up as `LOCAL HIDDEN` in the final
`libc.so` rather than `GLOBAL DEFAULT`, making them invisible to programs that
dynamically link against libc.

### What we know for certain

From `readelf -Ws libc.so`:
```
LOCAL  HIDDEN  memcpy    ← broken: not in .dynsym, programs can't resolve it
GLOBAL DEFAULT malloc    ← correct: in .dynsym
```

From building the minimal case (just `memcpy.lo`):
```
zig ld.lld --shared --export-dynamic -o t.so memcpy.lo
→ GLOBAL DEFAULT memcpy ✓   (works fine in isolation)
```

So the problem only appears when the full `LOBJS` set is linked together.

### What we ruled out

- **`libcompiler_rt.a`** — does not define memcpy. Confirmed with `nm`.
- **`libubsan_rt.a`** — does not define memcpy. Confirmed with `nm`.
- **`-fvisibility=hidden` in CFLAGS_AUTO** — `configure` does NOT set this
  when using zig cc (because `--dynamic-list` test fails, see below).
- **musl's `src/include/string.h`** — does not declare memcpy as hidden.
- **musl's ldso assembly** — no `.hidden memcpy` anywhere.
- **Multiple memcpy definitions** — musl's Makefile correctly excludes
  `src/string/memcpy.c` when `src/string/aarch64/memcpy.S` exists. Only the
  assembly version (GLOBAL DEFAULT) is compiled for aarch64.

### What we attempted

**rel=9–13**: patching `musl-build/Makefile` (the configure-generated wrapper)
to replace the libc.so link recipe with `zig ld.lld` directly, bypassing zig
cc's link-driver behavior. **This had no effect** — musl's configure creates a
thin wrapper (`srcdir = ..\ninclude $(srcdir)/Makefile`) with no recipe of its
own. The actual libc.so recipe lives in `../Makefile` (the source Makefile).
Our sed was patching the wrapper, not the source. All rel=9–13 builds silently
used the original `zig cc` link driver the entire time.

**`--dynamic-list` and `--version-script`**: neither fix the issue because lld
demotes `GLOBAL HIDDEN` → `LOCAL HIDDEN` before applying export rules (unlike
GNU ld which applies the dynamic list first). But since configure doesn't set
`-fvisibility=hidden` with zig cc, this demoting shouldn't matter — the assembly
symbols start as `GLOBAL DEFAULT`, not `GLOBAL HIDDEN`.

**`--export-dynamic`**: should export all `GLOBAL DEFAULT` symbols, but doesn't
help. Something in the full LOBJS set is introducing `HIDDEN` visibility for
memcpy even though no individual source declares it.

### Unresolved root cause

The exact mechanism by which the full musl link (but not the minimal
`memcpy.lo`-only link) produces `LOCAL HIDDEN` memcpy is still unknown.
Candidates that were not fully eliminated:

1. **zig cc's default `-fsanitize=...`**: zig cc enables a broad set of UBSan
   sanitizers by default. These produce references to sanitizer runtime symbols
   in every compiled object. It's possible some sanitizer-related reference to
   memcpy carries `STV_HIDDEN` visibility, causing lld to merge the visibility
   as HIDDEN (per ELF rules: most-restrictive visibility wins).

2. **Makefile sed targeting the wrong file**: As noted above, all our `zig
   ld.lld` patches were no-ops. The correct target is `../Makefile`. This means
   `zig cc` as link driver (with its ubsan_rt injection) was used in all builds,
   and the `zig ld.lld` code path was never actually tested end-to-end.

3. **Interaction between `-U_FORTIFY_SOURCE` coverage and the sanitizers**: The
   sed adds `-U_FORTIFY_SOURCE` to CFLAGS_AUTO, but zig cc's UBSan is separate
   from fortify source. The combination of UBSan + the zig cc link driver may
   introduce hidden symbol references in a way we did not isolate.

### Where to look next

To actually solve musl+mimalloc-in-libc, the path forward is:

1. **Fix the Makefile patch**: change `sed -i "s:...: Makefile"` to
   `sed -i "s:...: ../Makefile"` so the recipe actually gets replaced.

2. **Verify the recipe replacement worked** by grepping `../Makefile` for
   the zig ld.lld command after the sed.

3. **Test with zig ld.lld directly** (the sed now actually patching the right
   file) and check if memcpy exports correctly without ubsan_rt in the link.

4. **If step 3 still fails**, use `--trace-symbol=memcpy` in the zig ld.lld
   invocation to get lld's definitive account of which input introduced the
   HIDDEN visibility.

### Current workaround (rel=14 / mimalloc rel=1)

musl uses its built-in **mallocng** allocator. mimalloc is shipped as a
**separate shared library** (`libmimalloc.so.2`) and loaded via
`/etc/ld.so.preload`, which causes musl's dynamic linker to preload it before
any program starts. This gives all dynamically-linked programs mimalloc's
allocator without integrating it into libc.so.

## Key zig cc behaviors affecting system library builds

- Enables `_FORTIFY_SOURCE` by default → add `-U_FORTIFY_SOURCE` to CFLAGS_AUTO
- Enables broad UBSan sanitizers by default in link steps
- Injects `libubsan_rt.a` (without `--as-needed`) and `libcompiler_rt.a`
  (with `--as-needed`) in all shared library links via `zig cc` as link driver
- `--dynamic-list` check fails through `zig cc`'s `-Wl,` passthrough, so
  configure doesn't set `-fvisibility=hidden` — but lld natively supports
  `--dynamic-list` and `--version-script`
- lld demotes `GLOBAL HIDDEN` → `LOCAL HIDDEN` in the final output (ELF-spec
  compliant); GNU ld's `--dynamic-list` overrides this, lld's does not

## Testing memcpy export

```sh
# Check if memcpy is exported from libc.so (should show T memcpy)
nm -D libc.so | grep -E " T memcpy$| T memset$| T memmove$"

# Check if it's present but LOCAL HIDDEN (broken state)
readelf -Ws libc.so | grep memcpy

# Test that linked programs can find memcpy at runtime
zig cc -nostdinc -nostdlib -isystem /path/to/musl/include \
  -o test test.c crt1.o crti.o crtn.o libc.so \
  -dynamic-linker /usr/lib/ld-musl-aarch64.so.1
/usr/lib/ld-musl-aarch64.so.1 ./test
```
