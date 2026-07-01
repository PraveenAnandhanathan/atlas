/* C-ABI smoke test for libatlas_gvfs.so.
 *
 * This exercises the exact entry points the native GVFS backend
 * (libgvfsbackend-atlas.so) and the KIO worker link against, proving the
 * Rust C-ABI boundary works when called from real C — without needing a
 * running GNOME/KDE virtual-filesystem daemon.
 *
 * Build & run (from the repo root):
 *   cargo build -p atlas-gvfs
 *   cc crates/atlas-gvfs/ctest/abi_smoke.c -L target/debug -latlas_gvfs -o abi_smoke
 *   LD_LIBRARY_PATH=target/debug ./abi_smoke
 */
#include <stdio.h>
#include <string.h>
#include <stddef.h>

/* Declarations must match the #[no_mangle] extern "C" fns in gvfs.rs. */
extern int atlas_gvfs_mount_info(const char *uri, char **out);
extern void atlas_gvfs_free_string(char *ptr);

int main(void) {
    /* 1. Happy path: parse a URI, get mount JSON back, free it. */
    char *out = NULL;
    int rc = atlas_gvfs_mount_info("atlas://mlbox/research/data", &out);
    if (rc != 0 || out == NULL) {
        fprintf(stderr, "FAIL: mount_info rc=%d out=%p\n", rc, (void *)out);
        return 1;
    }
    printf("mount_info JSON: %s\n", out);
    if (strstr(out, "research") == NULL) {
        fprintf(stderr, "FAIL: mount JSON missing volume name\n");
        return 1;
    }
    atlas_gvfs_free_string(out);

    /* 2. Null URI must be rejected cleanly, not crash. */
    char *out2 = NULL;
    if (atlas_gvfs_mount_info(NULL, &out2) != -1) {
        fprintf(stderr, "FAIL: null uri was not rejected\n");
        return 1;
    }

    /* 3. A non-atlas scheme must be rejected. */
    char *out3 = NULL;
    if (atlas_gvfs_mount_info("file:///etc/passwd", &out3) != -1) {
        fprintf(stderr, "FAIL: bad scheme was not rejected\n");
        return 1;
    }

    /* 4. Freeing NULL is a documented no-op. */
    atlas_gvfs_free_string(NULL);

    printf("C-ABI smoke test PASSED\n");
    return 0;
}
