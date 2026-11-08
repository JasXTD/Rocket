/* Fixed guest compatibility only. These satisfy wasi-libc's linked references
 * WITHOUT any native host import. There are no descriptors or preopened paths.
 * The generated Wasm must have ZERO imports; new libc imports fail that gate.
 * These functions never read host state. Not compiled for a native target. */
#include <stdint.h>
#ifndef __wasm32__
#error This file must only be compiled inside the Wasm guest
#endif
#define W(name) __imported_wasi_snapshot_preview1_##name
#define BADF 8
#define NOTCAPABLE 76
#define NOSYS 52
int32_t W(environ_sizes_get)(uint32_t *count, uint32_t *bytes) {
    *count = 0; *bytes = 0; return 0;
}
int32_t W(environ_get)(uint32_t *entries, char *bytes) {
    (void)entries; (void)bytes; return 0;
}
int32_t W(clock_time_get)(int32_t id, int64_t precision, uint64_t *time) {
    (void)id; (void)precision; (void)time; return NOSYS;
}
int32_t W(fd_close)(int32_t fd) { (void)fd; return BADF; }
int32_t W(fd_fdstat_get)(int32_t fd, int32_t out) { (void)fd; (void)out; return BADF; }
int32_t W(fd_fdstat_set_flags)(int32_t fd, int32_t flags) { (void)fd; (void)flags; return BADF; }
int32_t W(fd_prestat_get)(int32_t fd, int32_t out) { (void)fd; (void)out; return BADF; }
int32_t W(fd_prestat_dir_name)(int32_t fd, int32_t out, int32_t len) { (void)fd; (void)out; (void)len; return BADF; }
int32_t W(fd_read)(int32_t fd, int32_t iov, int32_t len, int32_t out) { (void)fd; (void)iov; (void)len; (void)out; return BADF; }
int32_t W(fd_write)(int32_t fd, int32_t iov, int32_t len, int32_t out) { (void)fd; (void)iov; (void)len; (void)out; return BADF; }
int32_t W(fd_renumber)(int32_t from, int32_t to) { (void)from; (void)to; return BADF; }
int32_t W(fd_seek)(int32_t fd, int64_t offset, int32_t whence, int32_t out) { (void)fd; (void)offset; (void)whence; (void)out; return BADF; }
int32_t W(path_open)(int32_t fd, int32_t flags, int32_t path, int32_t len, int32_t oflags, int64_t rights, int64_t inheriting, int32_t fdflags, int32_t out) {
    (void)fd; (void)flags; (void)path; (void)len; (void)oflags; (void)rights; (void)inheriting; (void)fdflags; (void)out; return NOTCAPABLE;
}
int32_t W(path_remove_directory)(int32_t fd, int32_t path, int32_t len) { (void)fd; (void)path; (void)len; return NOTCAPABLE; }
int32_t W(path_unlink_file)(int32_t fd, int32_t path, int32_t len) { (void)fd; (void)path; (void)len; return NOTCAPABLE; }
int32_t W(path_rename)(int32_t fd, int32_t path, int32_t len, int32_t newfd, int32_t newpath, int32_t newlen) {
    (void)fd; (void)path; (void)len; (void)newfd; (void)newpath; (void)newlen; return NOTCAPABLE;
}
void W(proc_exit)(int32_t status) { (void)status; __builtin_trap(); }
int32_t W(sched_yield)(void) { return 0; }
