// First-party characterization probe, never a shipped helper. No Lua or user
// credentials are loaded. Each operation is bounded and chosen by the test host.
// App Sandbox is applied by macOS from this executable's signed entitlement.
#import <Foundation/Foundation.h>
#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <mach/mach.h>
#include <mach/mach_vm.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/resource.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/wait.h>
#include <unistd.h>

static const rlim_t memory_bytes = 128ULL * 1024 * 1024;
static const size_t allocation_bytes = 160ULL * 1024 * 1024;
static const char marker[] = "rocket-sandbox-synthetic";

static void emit(NSDictionary *value) {
    NSError *error = nil;
    NSData *data = [NSJSONSerialization dataWithJSONObject:value options:0 error:&error];
    if (!data || error || data.length > 4096 ||
        fwrite(data.bytes, 1, data.length, stdout) != data.length ||
        fputc('\n', stdout) == EOF || fflush(stdout) != 0) {
        _exit(91);
    }
}

static NSDictionary *file_probe(const char *path, bool write_mode) {
    errno = 0;
    int fd = open(path, (write_mode ? O_WRONLY | O_CREAT | O_EXCL : O_RDONLY) | O_NOFOLLOW, 0600);
    int saved = errno;
    bool success = false;
    if (fd >= 0) {
        char bytes[sizeof(marker)] = {0};
        ssize_t n = write_mode ? write(fd, marker, sizeof(marker)) : read(fd, bytes, sizeof(bytes));
        success = n == (ssize_t)sizeof(marker) && (write_mode || memcmp(bytes, marker, sizeof(marker)) == 0);
        saved = success ? 0 : errno;
        close(fd);
    }
    return @{ @"opened": fd >= 0 ? @YES : @NO, @"success": success ? @YES : @NO, @"errno": @(saved) };
}

static NSDictionary *limit_probe(int resource, rlim_t value) {
    struct rlimit old_limit, actual, desired = {value, value};
    if (getrlimit(resource, &old_limit) != 0) return @{ @"set": @NO, @"errno": @(errno) };
    // A probe must never raise a pre-existing restriction.
    if (old_limit.rlim_cur < desired.rlim_cur) desired.rlim_cur = old_limit.rlim_cur;
    if (old_limit.rlim_max < desired.rlim_max) desired.rlim_max = old_limit.rlim_max;
    errno = 0;
    int result = setrlimit(resource, &desired), saved = errno;
    bool checked = getrlimit(resource, &actual) == 0;
    return @{ @"set": result == 0 ? @YES : @NO, @"errno": @(saved), @"read_back": checked ? @YES : @NO,
              @"soft": @(checked ? actual.rlim_cur : 0), @"hard": @(checked ? actual.rlim_max : 0),
              @"requested": @(value) };
}

static void memory_probe(const char *kind) {
    // Test the APIs separately. A successful setrlimit is not proof that mmap
    // or Mach VM allocations are bounded. Never touch the 160 MiB reservation.
    int resource = strncmp(kind, "data-", 5) == 0 ? RLIMIT_DATA : RLIMIT_AS;
    NSDictionary *limit = limit_probe(resource, memory_bytes);
    if (![limit[@"set"] boolValue]) {
        emit(@{ @"limit": limit, @"attempted": @NO });
        return;
    }
    if (strstr(kind, "mach")) {
        mach_vm_address_t address = 0;
        kern_return_t result = mach_vm_allocate(mach_task_self(), &address, allocation_bytes, VM_FLAGS_ANYWHERE);
        if (result == KERN_SUCCESS) mach_vm_deallocate(mach_task_self(), address, allocation_bytes);
        emit(@{ @"limit": limit, @"attempted": @YES, @"allocated": result == KERN_SUCCESS ? @YES : @NO,
                @"native_code": @(result), @"bytes": @(allocation_bytes) });
    } else {
        int flags = MAP_ANON | (strstr(kind, "shared") ? MAP_SHARED : MAP_PRIVATE);
        errno = 0;
        void *address = mmap(NULL, allocation_bytes, PROT_READ | PROT_WRITE, flags, -1, 0);
        int saved = errno;
        bool allocated = address != MAP_FAILED;
        if (allocated) munmap(address, allocation_bytes);
        emit(@{ @"limit": limit, @"attempted": @YES, @"allocated": allocated ? @YES : @NO,
                @"errno": @(saved), @"bytes": @(allocation_bytes) });
    }
}

int main(int argc, const char **argv) {
    // Independent absolute deadline even if the characterization host crashes.
    // No handler replaces SIGALRM; a probe cannot live longer than five seconds.
    alarm(5);
    struct rlimit no_core = {0, 0}, cpu = {2, 3};
    if (geteuid() == 0 || setrlimit(RLIMIT_CORE, &no_core) || setrlimit(RLIMIT_CPU, &cpu)) return 90;
    @autoreleasepool {
        if (argc < 2) return 90;
        const char *operation = argv[1];
        if (strcmp(operation, "identity") == 0) {
            emit(@{ @"pid": @(getpid()), @"uid": @(geteuid()), @"home": NSHomeDirectory(),
                    @"bundle": NSBundle.mainBundle.bundleIdentifier ?: @"", @"protocol": @1 });
        } else if (strcmp(operation, "read") == 0 || strcmp(operation, "write") == 0) {
            if (argc != 3 || argv[2][0] != '/') return 90;
            emit(file_probe(argv[2], strcmp(operation, "write") == 0));
        } else if (strcmp(operation, "container-write") == 0 || strcmp(operation, "container-read") == 0) {
            NSString *home = NSHomeDirectory();
            NSString *bundle = NSBundle.mainBundle.bundleIdentifier;
            NSString *suffix = [NSString stringWithFormat:@"/Library/Containers/%@/Data", bundle];
            // Do not write in a real home if sandbox activation failed.
            if (!bundle || ![bundle hasPrefix:@"dev.rocket.sandbox-probe."] || ![home hasSuffix:suffix]) return 92;
            NSString *file = [home stringByAppendingPathComponent:@"rocket-sandbox-marker"];
            emit(file_probe(file.fileSystemRepresentation, strcmp(operation, "container-write") == 0));
        } else if (strcmp(operation, "network") == 0) {
            if (argc != 3) return 90;
            char *end = NULL;
            long port = strtol(argv[2], &end, 10);
            if (*end || port < 1 || port > 65535) return 90;
            int fd = socket(AF_INET, SOCK_STREAM, 0), saved = errno;
            bool connected = false;
            if (fd >= 0) {
                struct timeval timeout = {.tv_sec = 0, .tv_usec = 250000};
                if (setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &timeout, sizeof(timeout))) return 90;
                struct sockaddr_in address = {.sin_len = sizeof(address), .sin_family = AF_INET,
                    .sin_port = htons((uint16_t)port), .sin_addr.s_addr = htonl(INADDR_LOOPBACK)};
                connected = connect(fd, (struct sockaddr *)&address, sizeof(address)) == 0;
                saved = connected ? 0 : errno;
                close(fd);
            }
            emit(@{ @"socket": fd >= 0 ? @YES : @NO, @"connected": connected ? @YES : @NO, @"errno": @(saved) });
        } else if (strcmp(operation, "fork") == 0 || strcmp(operation, "limited-fork") == 0) {
            NSDictionary *limit = strcmp(operation, "limited-fork") == 0 ? limit_probe(RLIMIT_NPROC, 0) : @{};
            errno = 0;
            pid_t child = fork();
            int saved = errno, status = 0;
            if (child == 0) _exit(0);
            if (child > 0 && waitpid(child, &status, 0) != child) return 93;
            emit(@{ @"created": child > 0 ? @YES : @NO, @"errno": @(child < 0 ? saved : 0),
                    @"reaped": child > 0 && WIFEXITED(status) && WEXITSTATUS(status) == 0 ? @YES : @NO, @"limit": limit });
        } else if (strncmp(operation, "as-", 3) == 0 || strncmp(operation, "data-", 5) == 0) {
            if (!strstr(operation, "private") && !strstr(operation, "shared") && !strstr(operation, "mach")) return 90;
            memory_probe(operation);
        } else if (strcmp(operation, "wait") == 0) {
            emit(@{ @"ready": @YES, @"pid": @(getpid()) });
            // Fixed operation, no commands or script input. The parent tests Stop.
            for (;;) pause();
        } else {
            return 90;
        }
    }
    return 0;
}
