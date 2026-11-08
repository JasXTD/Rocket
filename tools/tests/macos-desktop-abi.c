#include <CoreFoundation/CoreFoundation.h>
#include <stddef.h>
// Validate the handwritten CoreFoundation ABI against the installed Apple SDK.
_Static_assert(sizeof(CFIndex) == sizeof(ptrdiff_t), "CFIndex");
_Static_assert(sizeof(CFTypeID) == sizeof(size_t), "CFTypeID");
_Static_assert(sizeof(Boolean) == 1, "Boolean");
_Static_assert(sizeof(CFStringEncoding) == 4, "CFStringEncoding");
_Static_assert(kCFStringEncodingUTF8 == 0x08000100, "UTF8");
_Static_assert(__builtin_types_compatible_p(__typeof__(&CFLocaleCopyPreferredLanguages), CFArrayRef (*)(void)), "preferred languages");
_Static_assert(__builtin_types_compatible_p(__typeof__(&CFLocaleCopyCurrent), CFLocaleRef (*)(void)), "current locale");
_Static_assert(__builtin_types_compatible_p(__typeof__(&CFLocaleGetIdentifier), CFStringRef (*)(CFLocaleRef)), "locale identifier");
_Static_assert(__builtin_types_compatible_p(__typeof__(&CFStringGetCString), Boolean (*)(CFStringRef, char *, CFIndex, CFStringEncoding)), "UTF8 copy");
int main(void) { return 0; }
