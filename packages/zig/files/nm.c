/* Minimal nm: print symbol names from ELF objects.
 * Enough for libtool's export-symbol extraction. */
#include <elf.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

static void nm64(const unsigned char *base, size_t sz) {
    const Elf64_Ehdr *eh = (const Elf64_Ehdr *)base;
    const Elf64_Shdr *sh = (const Elf64_Shdr *)(base + eh->e_shoff);
    for (int i = 0; i < eh->e_shnum; i++) {
        if (sh[i].sh_type != SHT_SYMTAB && sh[i].sh_type != SHT_DYNSYM)
            continue;
        const Elf64_Sym *sym = (const Elf64_Sym *)(base + sh[i].sh_offset);
        const char *str = (const char *)(base + sh[sh[i].sh_link].sh_offset);
        int n = sh[i].sh_size / sh[i].sh_entsize;
        for (int j = 0; j < n; j++) {
            if (sym[j].st_name == 0) continue;
            const char *name = str + sym[j].st_name;
            char type = '?';
            unsigned char bind = ELF64_ST_BIND(sym[j].st_info);
            unsigned char stype = ELF64_ST_TYPE(sym[j].st_info);
            if (sym[j].st_shndx == SHN_UNDEF) type = 'U';
            else if (sym[j].st_shndx == SHN_ABS) type = 'A';
            else if (sym[j].st_shndx == SHN_COMMON) type = 'C';
            else if (stype == STT_FUNC || stype == STT_GNU_IFUNC) type = 'T';
            else if (stype == STT_OBJECT) type = 'D';
            else if (stype == STT_NOTYPE) type = 'T';
            if (bind == STB_LOCAL && type != 'U') type = type - 'A' + 'a';
            if (sym[j].st_shndx == SHN_UNDEF)
                printf("                 %c %s\n", type, name);
            else
                printf("%016lx %c %s\n", (unsigned long)sym[j].st_value, type, name);
        }
    }
}

int main(int argc, char **argv) {
    for (int i = 1; i < argc; i++) {
        if (argv[i][0] == '-') continue;  /* skip flags */
        int fd = open(argv[i], O_RDONLY);
        if (fd < 0) { perror(argv[i]); continue; }
        struct stat st;
        fstat(fd, &st);
        unsigned char *base = mmap(NULL, st.st_size, PROT_READ, MAP_PRIVATE, fd, 0);
        close(fd);
        if (base == MAP_FAILED) { perror("mmap"); continue; }
        if (st.st_size >= (off_t)sizeof(Elf64_Ehdr) && memcmp(base, ELFMAG, SELFMAG) == 0)
            nm64(base, st.st_size);
        munmap(base, st.st_size);
    }
    return 0;
}
