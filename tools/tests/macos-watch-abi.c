// Check hand-written Rust declarations against the installed Apple SDK.
#include <CoreServices/CoreServices.h>
#include <dispatch/dispatch.h>
#include <stddef.h>
#include <stdint.h>
#define CHECK_TYPE(expression, type) _Static_assert(__builtin_types_compatible_p(__typeof__(expression), type), #expression " ABI")
_Static_assert(__builtin_types_compatible_p(FSEventStreamCallback, void (*)(ConstFSEventStreamRef,void *,size_t,void *,const FSEventStreamEventFlags *,const FSEventStreamEventId *)),"callback ABI");
CHECK_TYPE(&FSEventStreamCreate, FSEventStreamRef (*)(CFAllocatorRef,FSEventStreamCallback,FSEventStreamContext *,CFArrayRef,FSEventStreamEventId,CFTimeInterval,FSEventStreamCreateFlags));
CHECK_TYPE(&FSEventStreamSetDispatchQueue, void (*)(FSEventStreamRef,dispatch_queue_t));
CHECK_TYPE(&FSEventStreamStart, Boolean (*)(FSEventStreamRef));
CHECK_TYPE(&FSEventStreamStop, void (*)(FSEventStreamRef));
CHECK_TYPE(&FSEventStreamInvalidate, void (*)(FSEventStreamRef));
CHECK_TYPE(&FSEventStreamRelease, void (*)(FSEventStreamRef));
CHECK_TYPE(&CFStringCreateWithBytes, CFStringRef (*)(CFAllocatorRef,const UInt8 *,CFIndex,CFStringEncoding,Boolean));
CHECK_TYPE(&CFArrayCreate, CFArrayRef (*)(CFAllocatorRef,const void **,CFIndex,const CFArrayCallBacks *));
CHECK_TYPE(&dispatch_queue_create, dispatch_queue_t (*)(const char *,dispatch_queue_attr_t));
CHECK_TYPE(&dispatch_sync_f, void (*)(dispatch_queue_t,void *,dispatch_function_t));
CHECK_TYPE(&dispatch_release, void (*)(dispatch_object_t));
_Static_assert(sizeof(FSEventStreamContext)==40 && offsetof(FSEventStreamContext,info)==8 && offsetof(FSEventStreamContext,copyDescription)==32,"context ABI");
_Static_assert(sizeof(FSEventStreamEventId)==8 && sizeof(FSEventStreamEventFlags)==4 && sizeof(Boolean)==1,"event ABI");
_Static_assert(kFSEventStreamEventIdSinceNow==UINT64_MAX,"since now");
_Static_assert(kFSEventStreamCreateFlagNoDefer==2 && kFSEventStreamCreateFlagWatchRoot==4 && kFSEventStreamCreateFlagFileEvents==16,"create flags");
_Static_assert(kFSEventStreamEventFlagMustScanSubDirs==1 && kFSEventStreamEventFlagUserDropped==2 && kFSEventStreamEventFlagKernelDropped==4 && kFSEventStreamEventFlagEventIdsWrapped==8,"loss flags");
_Static_assert(kFSEventStreamEventFlagHistoryDone==16 && kFSEventStreamEventFlagRootChanged==32 && kFSEventStreamEventFlagMount==64 && kFSEventStreamEventFlagUnmount==128,"root flags");
int main(void) { return 0; }
