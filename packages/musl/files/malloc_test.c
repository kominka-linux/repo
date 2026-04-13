#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <malloc.h>

int main(void) {
    /* basic alloc/free */
    void *p = malloc(1024);
    if (!p) { fprintf(stderr, "malloc failed\n"); return 1; }
    memset(p, 0x42, 1024);
    size_t usable = malloc_usable_size(p);
    printf("malloc(1024)  ptr=%p  usable=%zu\n", p, usable);
    if (usable < 1024) { fprintf(stderr, "usable size too small\n"); return 1; }
    free(p);

    /* calloc */
    int *arr = calloc(256, sizeof(int));
    if (!arr) { fprintf(stderr, "calloc failed\n"); return 1; }
    for (int i = 0; i < 256; i++) arr[i] = i;
    printf("calloc(256,4) ptr=%p  arr[255]=%d\n", (void *)arr, arr[255]);
    free(arr);

    /* realloc */
    char *s = malloc(8);
    if (!s) { fprintf(stderr, "realloc-base malloc failed\n"); return 1; }
    strcpy(s, "hello");
    s = realloc(s, 64);
    if (!s) { fprintf(stderr, "realloc failed\n"); return 1; }
    strcat(s, " mimalloc");
    printf("realloc       str=%s\n", s);
    free(s);

    /* aligned_alloc */
    void *a = aligned_alloc(4096, 4096);
    if (!a) { fprintf(stderr, "aligned_alloc failed\n"); return 1; }
    if ((size_t)a % 4096 != 0) { fprintf(stderr, "alignment wrong: %p\n", a); return 1; }
    printf("aligned_alloc ptr=%p (4096-aligned)\n", a);
    free(a);

    printf("OK\n");
    return 0;
}
