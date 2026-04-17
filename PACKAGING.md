# Build Procedure for Kominka Packages

Reference guide for coding agents iterating on PKGBUILDs.

---

## Minimal Builds

Only build what is strictly required for runtime. Skip docs, man pages, tests, and NLS
unless the package is specifically a documentation package.

- Pass `--disable-nls`, `--disable-dependency-tracking`, `--disable-tests` to configure
  where available.
- Remove `doc`, `po`, `tests` from `SUBDIRS` in generated Makefiles where present.
- For binutils-style packages: build only the specific targets needed (e.g.
  `make -C binutils strip-new nm-new`) rather than `make all`.
- Pass `MAKEINFO=true PERL=true` to suppress doc/man generation failures when those
  tools are not installed.

---

## System cc vs Raw zig cc

**Always use system `cc`/`c++` (the zig wrappers at `/usr/bin/cc` and `/usr/bin/c++`),
not raw `zig cc`.**

Raw `zig cc` without `-target ARCH-linux-musl` may generate binaries for the wrong
architecture, causing SIGILL at runtime. The wrappers bake in the correct target.

`pm.ysh` sets `ENV.CC=cc`, `ENV.CXX=c++`, `ENV.AR=ar`, `ENV.NM=nm`, `ENV.RANLIB=ranlib`
as defaults for every build proc. Do not add redundant `setglobal ENV.CC = 'cc'` lines
in PKGBUILDs. Only `musl` may use raw `zig cc` (it sets its own target explicitly).

For packages that pass CC as a make variable rather than an environment variable, use
`ENV => get('CC', 'cc')` to read the already-set environment value.

---

## AC_SUBST_FILE Placeholders (seed awk bug)

seed's awk cannot execute the `getline < file` substitution pattern used by
`config.status` for `AC_SUBST_FILE` variables. These variables survive as literal
`@VAR@` strings in generated Makefiles, causing `make: Makefile:NNN: missing separator`.

**Fix after every `./configure`:**

```sh
# For @serialization_dependencies@: insert file contents, then delete placeholder.
seed sed -i "/@serialization_dependencies@/r serdep.tmp" Makefile

# For @target_makefile_frag@: replace with the appropriate content.
sed -i 's|^@target_makefile_frag@$|CXXFLAGS_FOR_TARGET += -D_GNU_SOURCE|' Makefile

# For @MAINT_MAKEFILE@: always delete (maintMakefile absent in release tarballs).
sed -i '/^@MAINT_MAKEFILE@$/d' Makefile

# Delete all remaining bare @VAR@ placeholder lines.
sed -i '/^@[a-zA-Z_][a-zA-Z0-9_]*@$/d' Makefile

# Prevent Makefile self-regeneration from undoing the patches (see below).
touch Makefile
```

Apply analogous fixes to any sub-package Makefiles (libiberty, bfd, binutils, etc.).

---

## Autotools Timestamp Cascade

In release tarballs, derived files (`aclocal.m4`, `Makefile.in`) are often timestamped
newer than their outputs, causing make to try to re-run `aclocal`, `autoconf`, or
`automake` — none of which are installed.

**Fix:** pass these overrides on every `make` invocation:

```sh
make ACLOCAL=true AUTOCONF=true AUTOMAKE=true MAKEINFO=true PERL=true
```

These propagate to all recursive makes via `MAKEFLAGS`. `-o FILE` flags do NOT
propagate recursively, so always use the `VAR=true` form.

---

## Makefile Self-Regeneration

Generated Makefiles contain a rule:
```makefile
Makefile: Makefile.in config.status
	./config.status Makefile
```
If `config.status` is newer than the patched `Makefile`, make will regenerate it,
discarding your sed patches. **Always run `touch Makefile` after patching** to bump
its timestamp past `config.status`.

---

## Sub-package Makefiles Created Lazily

In GNU multi-package trees (binutils, gcc, etc.), top-level `./configure` creates only
the top-level `Makefile`. Sub-package Makefiles (`libiberty/Makefile`, `bfd/Makefile`,
etc.) are created lazily when `make configure-XXX` runs.

**Pattern:**
```sh
# 1. Patch top-level Makefile.
sed -i ...
touch Makefile

# 2. Explicitly pre-configure the sub-packages you need.
make configure-libiberty configure-bfd configure-binutils \
  ACLOCAL=true AUTOCONF=true AUTOMAKE=true

# 3. Patch sub-package Makefiles.
for mf in libiberty/Makefile bfd/Makefile binutils/Makefile ... {
    if test -f $mf {
        sed -i '/^@[a-zA-Z_][a-zA-Z0-9_]*@$/d' $mf
        touch $mf
    }
}

# 4. Build only what you need.
make all-libiberty all-bfd ACLOCAL=true AUTOCONF=true AUTOMAKE=true MAKEINFO=true
make -C binutils strip-new nm-new ACLOCAL=true AUTOCONF=true AUTOMAKE=true MAKEINFO=true PERL=true
```

---

## Autoconf Cache Pre-Population

zig cc (strict Clang) rejects implicit function declarations, causing false negatives in
`AC_CHECK_FUNCS` tests. Pre-populate `config.cache` before running `./configure`:

```sh
builtin printf '%s\n' \
    'ac_cv_func_foo=yes' \
    'ac_cv_func_bar=yes' \
    > config.cache
./configure --cache-file=config.cache ...
```

Also add `--cache-file=config.cache` to the configure invocation.

---

## C++ Include Order: libc++ vs musl Headers

**Symptom**: `<cstring> tried including <string.h> but didn't find libc++'s <string.h>
header. The header search paths should contain the C++ Standard Library headers before
any C Standard Library.`

**Root cause**: zig c++ adds its libc++ headers as system includes (`-isystem`), but any
user `-I` flags are searched *before* system includes. The `/usr/bin/c++` wrapper
appends `-I/usr/include` (user include), so musl's `/usr/include/string.h` is found
before zig's libc++ `string.h` wrapper.

**Fix**: The `/usr/bin/c++` wrapper must use `-isystem /usr/include` instead of
`-I/usr/include`. This is done in the zig PKGBUILD and applied to the builder image
via Dockerfile:

```dockerfile
RUN sed -i 's| -I/usr/include| -isystem /usr/include|g' /usr/bin/c++
```

## cmake Bootstrap

cmake's `./configure` is a bootstrap shell script, not autoconf. It accepts `CC=`
and `CXX=` as positional arguments (before `--`) and also reads CXX from the
environment. Bootstrap errors go to `Bootstrap.cmk/cmake_bootstrap.log`.

```sh
./configure --prefix=/usr --parallel=$nproc -- -DCMAKE_USE_OPENSSL=OFF
```

---

## Broken Mkdep from Remote Repo

Packages in the remote repo (R2) may have been built with an old, broken zig cc
setup (no `-target` flag), producing binaries that crash with SIGILL at runtime.
pm skips reinstalling mkdeps that are already present in `sys_db/`, so the broken
binary stays even after the PKGBUILD is fixed.

**Pattern**: detect breakage at the top of `build()`, then rebuild and reinstall:

```ysh
proc build(dest) {
    if ! m4 --version >/dev/null 2>&1 {
        # m4 from the remote repo is broken (SIGILL). Rebuild from source so
        # the local cache gets a working binary, then reinstall it.
        pm b m4
        pm i m4
    }
    ...
}
```

`pm b m4` always rebuilds from source (overwriting the cached tarball). `pm i m4`
then installs from the freshly-built local cache. In the normal `build-all.sh`
flow the prior wave already built the fixed binary, so this is a safety fallback.

---

## seed head -n Incompatibility

seed's `head` does not support the POSIX `-n` option. Use the traditional form:

```sh
head -1       # correct: first line
head -5       # correct: first 5 lines
head -n 1     # WRONG: seed head does not support -n
head -n1      # WRONG: seed head does not support -n
```

If generated or bundled scripts use `head -n1`, patch them before they run:

```sh
sed -i 's/head -n1/head -1/g' script.sh
```

---

## seed find -exec {} + Unreliability

`seed find . -name Makefile -exec sed -i ... {} +` may not work correctly. Prefer an
explicit ysh loop over known paths:

```ysh
for mf in known/path/Makefile other/path/Makefile {
    if test -f $mf {
        sed -i ... $mf
        touch $mf
    }
}
```
