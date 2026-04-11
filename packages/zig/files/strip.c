/* Minimal strip: remove debug/symbol sections from ELF files.
 * Keeps .dynsym/.dynstr (needed at runtime). Modifies files in place.
 * Truncates the file after the last LOAD segment for real size savings. */
#include <elf.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

static size_t strip64(unsigned char *base, size_t sz) {
    Elf64_Ehdr *eh = (Elf64_Ehdr *)base;
    if (eh->e_shoff == 0 || eh->e_shnum == 0) return sz;
    if (eh->e_shoff + eh->e_shnum * sizeof(Elf64_Shdr) > sz) return sz;

    Elf64_Shdr *sh = (Elf64_Shdr *)(base + eh->e_shoff);
    Elf64_Phdr *ph = (Elf64_Phdr *)(base + eh->e_phoff);
    const char *shstrtab = NULL;
    if (eh->e_shstrndx < eh->e_shnum)
        shstrtab = (const char *)(base + sh[eh->e_shstrndx].sh_offset);

    /* Zero out strippable sections. */
    for (int i = 0; i < eh->e_shnum; i++) {
        if (sh[i].sh_offset + sh[i].sh_size > sz) continue;
        if (!shstrtab) continue;
        const char *name = shstrtab + sh[i].sh_name;

        /* Keep sections needed at runtime. */
        if (sh[i].sh_type == SHT_DYNSYM) continue;
        if (sh[i].sh_flags & SHF_ALLOC) continue;
        if (strcmp(name, ".dynstr") == 0) continue;
        if (strcmp(name, ".gnu.hash") == 0) continue;
        if (strcmp(name, ".hash") == 0) continue;
        if (strcmp(name, ".shstrtab") == 0) continue;

        /* Strip everything else that's debug/symbol data. */
        int strip = 0;
        if (sh[i].sh_type == SHT_SYMTAB) strip = 1;
        if (sh[i].sh_type == SHT_NOTE) strip = 1;
        if (strncmp(name, ".debug", 6) == 0) strip = 1;
        if (strncmp(name, ".zdebug", 7) == 0) strip = 1;
        if (strcmp(name, ".strtab") == 0) strip = 1;
        if (strcmp(name, ".comment") == 0) strip = 1;

        if (strip && sh[i].sh_size > 0) {
            memset(base + sh[i].sh_offset, 0, sh[i].sh_size);
            sh[i].sh_size = 0;
            sh[i].sh_type = SHT_NOBITS;
        }
    }

    /* Find the truncation point: end of last non-stripped section or LOAD segment. */
    size_t end = 0;
    for (int i = 0; i < eh->e_phnum; i++) {
        size_t seg_end = ph[i].p_offset + ph[i].p_filesz;
        if (seg_end > end) end = seg_end;
    }
    for (int i = 0; i < eh->e_shnum; i++) {
        if (sh[i].sh_type == SHT_NOBITS) continue;
        if (sh[i].sh_size == 0) continue;
        size_t sec_end = sh[i].sh_offset + sh[i].sh_size;
        if (sec_end > end) end = sec_end;
    }
    /* Keep section header table. */
    size_t sh_end = eh->e_shoff + eh->e_shnum * sizeof(Elf64_Shdr);
    if (sh_end > end) end = sh_end;

    return end > 0 ? end : sz;
}

int main(int argc, char **argv) {
    for (int i = 1; i < argc; i++) {
        if (argv[i][0] == '-') {
            if (strcmp(argv[i], "-o") == 0 || strcmp(argv[i], "-R") == 0 ||
                strcmp(argv[i], "-N") == 0 || strcmp(argv[i], "-K") == 0)
                i++;
            continue;
        }

        int fd = open(argv[i], O_RDWR);
        if (fd < 0) { perror(argv[i]); continue; }
        struct stat st;
        fstat(fd, &st);
        if (st.st_size < (off_t)sizeof(Elf64_Ehdr)) { close(fd); continue; }

        unsigned char *base = mmap(NULL, st.st_size, PROT_READ|PROT_WRITE,
                                    MAP_SHARED, fd, 0);
        if (base == MAP_FAILED) { perror("mmap"); close(fd); continue; }

        size_t new_size = st.st_size;
        if (memcmp(base, ELFMAG, SELFMAG) == 0)
            new_size = strip64(base, st.st_size);

        munmap(base, st.st_size);

        if ((size_t)new_size < (size_t)st.st_size)
            ftruncate(fd, new_size);

        close(fd);
    }
    return 0;
}
