/*
 * float128_shim.c — stub definitions for 128-bit float ops (aarch64).
 *
 * Purpose: prevent compiler_rt from being linked into libc.so. zig cc adds
 * compiler_rt with --as-needed; when these symbols are already defined here,
 * compiler_rt's object is NOT pulled in. This is critical because compiler_rt's
 * object also defines memcpy/memset/memmove as WEAK HIDDEN, which poisons
 * musl's GLOBAL DEFAULT assembly exports via ELF visibility merging.
 *
 * These stubs trap if actually called. In practice, 128-bit float math
 * (sinl, cosl, printf %Lf, etc.) is never used on a minimal server distro.
 * Compiled with -fvisibility=hidden so they don't appear in .dynsym.
 */

/* Use opaque 128-bit integer representation to avoid long double arithmetic
 * that would recursively call these same functions. */
typedef struct { unsigned long long lo, hi; } f128;
typedef struct { unsigned int lo; } f32;
typedef struct { unsigned long long lo; } f64;

#define STUB __attribute__((visibility("hidden"), noinline))
#define TRAP() __builtin_trap()

STUB f128 __addtf3(f128 a, f128 b) { TRAP(); }
STUB f128 __subtf3(f128 a, f128 b) { TRAP(); }
STUB f128 __multf3(f128 a, f128 b) { TRAP(); }
STUB f128 __divtf3(f128 a, f128 b) { TRAP(); }
STUB f128 __negtf2(f128 a)         { TRAP(); }

STUB int  __eqtf2(f128 a, f128 b)    { TRAP(); }
STUB int  __netf2(f128 a, f128 b)    { TRAP(); }
STUB int  __lttf2(f128 a, f128 b)    { TRAP(); }
STUB int  __letf2(f128 a, f128 b)    { TRAP(); }
STUB int  __gttf2(f128 a, f128 b)    { TRAP(); }
STUB int  __getf2(f128 a, f128 b)    { TRAP(); }
STUB int  __unordtf2(f128 a, f128 b) { TRAP(); }

STUB f128 __extenddftf2(f64 a)  { TRAP(); }
STUB f128 __extendsftf2(f32 a)  { TRAP(); }
STUB f64  __trunctfdf2(f128 a)  { TRAP(); }
STUB f32  __trunctfsf2(f128 a)  { TRAP(); }

STUB int              __fixtfsi(f128 a)     { TRAP(); }
STUB long long        __fixtfdi(f128 a)     { TRAP(); }
STUB unsigned int     __fixunstfsi(f128 a)  { TRAP(); }
STUB unsigned long long __fixunstfdi(f128 a){ TRAP(); }
STUB f128 __floatsitf(int a)                { TRAP(); }
STUB f128 __floatditf(long long a)          { TRAP(); }
STUB f128 __floatunsitf(unsigned int a)     { TRAP(); }
STUB f128 __floatunditf(unsigned long long a){ TRAP(); }

/* Complex multiply (C99 complex.h). Used by musl's <complex.h> functions.
 * Signatures use void* to avoid complex ABI details — these stubs only trap. */
STUB void __mulsc3(void) { TRAP(); }  /* float _Complex multiply */
STUB void __muldc3(void) { TRAP(); }  /* double _Complex multiply */
STUB void __multc3(void) { TRAP(); }  /* long double _Complex multiply */
