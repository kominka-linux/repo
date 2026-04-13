/*
 * float128_shim.c — 128-bit float ops for musl libc (aarch64 only).
 *
 * On aarch64, long double == __float128 == IEEE 754 quad precision.
 * musl's math library references these ops; compiler_rt normally provides
 * them but also defines WEAK HIDDEN memcpy which poisons musl's exports.
 * This shim provides only the float ops with HIDDEN visibility.
 */
typedef long double f128;

__attribute__((visibility("hidden"))) f128 __addtf3(f128 a, f128 b) { return a + b; }
__attribute__((visibility("hidden"))) f128 __subtf3(f128 a, f128 b) { return a - b; }
__attribute__((visibility("hidden"))) f128 __multf3(f128 a, f128 b) { return a * b; }
__attribute__((visibility("hidden"))) f128 __divtf3(f128 a, f128 b) { return a / b; }
__attribute__((visibility("hidden"))) f128 __negtf2(f128 a)         { return -a; }

__attribute__((visibility("hidden"))) int __eqtf2(f128 a, f128 b)    { return a == b ? 0 : 1; }
__attribute__((visibility("hidden"))) int __netf2(f128 a, f128 b)    { return a != b; }
__attribute__((visibility("hidden"))) int __lttf2(f128 a, f128 b)    { return a <  b ? -1 : 0; }
__attribute__((visibility("hidden"))) int __letf2(f128 a, f128 b)    { return a <= b ? -1 : 0; }
__attribute__((visibility("hidden"))) int __gttf2(f128 a, f128 b)    { return a >  b ?  1 : 0; }
__attribute__((visibility("hidden"))) int __getf2(f128 a, f128 b)    { return a >= b ?  1 : 0; }
__attribute__((visibility("hidden"))) int __unordtf2(f128 a, f128 b) { return a != a || b != b; }

__attribute__((visibility("hidden"))) f128 __extenddftf2(double a)  { return (f128)a; }
__attribute__((visibility("hidden"))) f128 __extendsftf2(float a)   { return (f128)a; }
__attribute__((visibility("hidden"))) double __trunctfdf2(f128 a)   { return (double)a; }
__attribute__((visibility("hidden"))) float  __trunctfsf2(f128 a)   { return (float)a; }

__attribute__((visibility("hidden"))) int           __fixtfsi(f128 a) { return (int)a; }
__attribute__((visibility("hidden"))) long long     __fixtfdi(f128 a) { return (long long)a; }
__attribute__((visibility("hidden"))) unsigned int  __fixunstfsi(f128 a) { return (unsigned int)a; }
__attribute__((visibility("hidden"))) unsigned long long __fixunstfdi(f128 a) { return (unsigned long long)a; }
__attribute__((visibility("hidden"))) f128 __floatsitf(int a)            { return (f128)a; }
__attribute__((visibility("hidden"))) f128 __floatditf(long long a)      { return (f128)a; }
__attribute__((visibility("hidden"))) f128 __floatunsitf(unsigned int a) { return (f128)a; }
__attribute__((visibility("hidden"))) f128 __floatunditf(unsigned long long a) { return (f128)a; }
