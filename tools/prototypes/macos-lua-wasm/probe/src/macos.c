// First-party helper guard. Uses only public SDK declarations; no downloaded
// implementation. App Sandbox is a second boundary around the import-free VM.
#include <Security/Security.h>
#include <CoreFoundation/CoreFoundation.h>
#include <mach/mach.h>
#include <libproc.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <stdint.h>
#include <unistd.h>

int rocket_close_inherited(void) {
    // Enumerate open descriptors, including ones above the current soft limit.
    // Called before starting threads or asking Security.framework for identity.
    struct proc_fdinfo fds[4096];
    int bytes = proc_pidinfo(getpid(), PROC_PIDLISTFDS, 0, fds, sizeof(fds));
    if (bytes <= 0 || bytes >= (int)sizeof(fds) || bytes % sizeof(fds[0])) return -1;
    for (size_t i = 0; i < (size_t)bytes / sizeof(fds[0]); ++i) {
        if (fds[i].proc_fd >= 4 && close(fds[i].proc_fd) != 0) return -1;
    }
    return 0;
}

int rocket_attest(void) {
    SecCodeRef self = NULL;
    CFDictionaryRef info = NULL;
    int ok = 0;
    if (SecCodeCopySelf(kSecCSDefaultFlags, &self) != errSecSuccess) goto end;
    if (SecCodeCheckValidity(self, kSecCSDefaultFlags, NULL) != errSecSuccess) goto end;
    if (SecCodeCopySigningInformation((SecStaticCodeRef)self,
            kSecCSSigningInformation | kSecCSDynamicInformation, &info) != errSecSuccess) goto end;
    CFTypeRef ent = CFDictionaryGetValue(info, kSecCodeInfoEntitlementsDict);
    CFTypeRef flags = CFDictionaryGetValue(info, kSecCodeInfoFlags);
    if (!ent || CFGetTypeID(ent) != CFDictionaryGetTypeID() ||
        CFDictionaryGetCount((CFDictionaryRef)ent) != 1 ||
        CFDictionaryGetValue((CFDictionaryRef)ent, CFSTR("com.apple.security.app-sandbox")) != kCFBooleanTrue)
        goto end;
    int32_t value = 0;
    if (!flags || CFGetTypeID(flags) != CFNumberGetTypeID() ||
        !CFNumberGetValue((CFNumberRef)flags, kCFNumberSInt32Type, &value) ||
        !(value & kSecCodeSignatureRuntime)) goto end;
    ok = 1;
end:
    if (info) CFRelease(info);
    if (self) CFRelease(self);
    return ok;
}

static int lower_limit(int resource, rlim_t ceiling) {
    struct rlimit old, value, actual;
    if (getrlimit(resource, &old)) return -1;
    value.rlim_max = old.rlim_max < ceiling ? old.rlim_max : ceiling;
    value.rlim_cur = old.rlim_cur < value.rlim_max ? old.rlim_cur : value.rlim_max;
    if (setrlimit(resource, &value) || getrlimit(resource, &actual)) return -1;
    return actual.rlim_cur <= ceiling && actual.rlim_max <= ceiling ? 0 : -1;
}

int rocket_limits(void) {
    return lower_limit(RLIMIT_CORE, 0) || lower_limit(RLIMIT_NPROC, 0) ||
        lower_limit(RLIMIT_CPU, 3) || lower_limit(RLIMIT_FSIZE, 0) ||
        lower_limit(RLIMIT_NOFILE, 64) ? -1 : 0;
}

int rocket_memory(uint64_t *bytes) {
    task_vm_info_data_t info = {0};
    mach_msg_type_number_t count = TASK_VM_INFO_COUNT;
    if (task_info(mach_task_self(), TASK_VM_INFO, (task_info_t)&info, &count) != KERN_SUCCESS ||
        count < TASK_VM_INFO_REV1_COUNT) return -1;
    // RSS includes shared pages; physical footprint also accounts for compressed
    // private allocations. This is a monitor sample, not a kernel allocation cap.
    *bytes = info.resident_size > info.phys_footprint ? info.resident_size : info.phys_footprint;
    return 0;
}
