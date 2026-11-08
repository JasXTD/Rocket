// Compile against the installed Apple SDK before running the native Rust tests.
// These checks make handwritten FFI types/status values independently verifiable.
#include <CoreFoundation/CoreFoundation.h>
#include <Security/Security.h>
#include <stdint.h>
#include <stdio.h>
#define SIGNATURE(name, type) _Static_assert(_Generic(&(name), type: 1, default: 0), #name)
_Static_assert(sizeof(Boolean) == 1, "Boolean");
_Static_assert(sizeof(OSStatus) == 4, "OSStatus");
_Static_assert(sizeof(CFIndex) == sizeof(intptr_t), "CFIndex");
_Static_assert(kCFStringEncodingUTF8 == 0x08000100, "UTF8");
_Static_assert(errSecItemNotFound == -25300, "item missing");
_Static_assert(errSecDuplicateItem == -25299, "add-only duplicate");
_Static_assert(errSecAuthFailed == -25293, "auth denied");
_Static_assert(errSecInteractionNotAllowed == -25308, "prompt refused");
_Static_assert(errSecNoSuchKeychain == -25294, "Keychain missing");
_Static_assert(errSecNoDefaultKeychain == -25307, "default missing");
SIGNATURE(SecKeychainCopyDefault, OSStatus (*)(SecKeychainRef *));
SIGNATURE(SecKeychainGetUserInteractionAllowed, OSStatus (*)(Boolean *));
SIGNATURE(SecKeychainSetUserInteractionAllowed, OSStatus (*)(Boolean));
SIGNATURE(SecItemCopyMatching, OSStatus (*)(CFDictionaryRef, CFTypeRef *));
SIGNATURE(SecItemAdd, OSStatus (*)(CFDictionaryRef, CFTypeRef *));
SIGNATURE(SecAccessCreate, OSStatus (*)(CFStringRef, CFArrayRef, SecAccessRef *));
SIGNATURE(SecTrustedApplicationCreateFromPath, OSStatus (*)(const char *, SecTrustedApplicationRef *));
SIGNATURE(SecRandomCopyBytes, int (*)(SecRandomRef, size_t, void *));
SIGNATURE(SecKeychainCreate, OSStatus (*)(const char *, UInt32, const void *, Boolean, SecAccessRef, SecKeychainRef *));
SIGNATURE(SecKeychainDelete, OSStatus (*)(SecKeychainRef));
SIGNATURE(SecKeychainUnlock, OSStatus (*)(SecKeychainRef, UInt32, const void *, Boolean));
SIGNATURE(SecKeychainLock, OSStatus (*)(SecKeychainRef));
SIGNATURE(SecKeychainCopySearchList, OSStatus (*)(CFArrayRef *));
SIGNATURE(SecKeychainSetSearchList, OSStatus (*)(CFArrayRef));
int main(void) {
    puts("PASS Apple SDK credential ABI and status assertions");
    return 0;
}
