// Test-only observation of the exact packaged app. Does not replace Cocoa results.
// No user text, word suggestions, learned words or credentials are recorded.
#import <AppKit/AppKit.h>
#import <objc/runtime.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
static NSRange (*original)(id, SEL, NSString *, NSInteger, NSString *, BOOL, NSInteger, NSInteger *);
static NSRange observed(id self, SEL selector, NSString *text, NSInteger start,
                        NSString *language, BOOL wrap, NSInteger tag, NSInteger *words) {
    double begin = CFAbsoluteTimeGetCurrent();
    NSRange result = original(self, selector, text, start, language, wrap, tag, words);
    @autoreleasepool {
        NSDictionary *event = @{
            @"event":@"spell_native_return", @"text_units":@(text.length),
            @"at_ms":@([[NSDate date] timeIntervalSince1970] * 1000.0),
            @"start":@(start), @"language":language ?: @"",
            @"not_found":@(result.location == NSNotFound), @"length":@(result.length),
            @"location":@(result.location), @"wrap":@(wrap),
            @"word_count_requested":@(words != NULL),
            @"elapsed_ms":@((CFAbsoluteTimeGetCurrent() - begin) * 1000.0)
        };
        NSData *json = [NSJSONSerialization dataWithJSONObject:event options:0 error:nil];
        if (json) { fputs("ROCKET_SPELL_TRACE ", stderr); fwrite(json.bytes, 1, json.length, stderr); fputc('\n', stderr); fflush(stderr); }
    }
    return result;
}
__attribute__((constructor)) static void install(void) {
    if (!getenv("GITHUB_ACTIONS") || strcmp(getenv("GITHUB_ACTIONS"), "true") ||
        !getenv("ROCKET_SPELL_OBSERVE") || strcmp(getenv("ROCKET_SPELL_OBSERVE"), "1") ||
        strcmp(getprogname(), "rocket-mcp")) return;
    @autoreleasepool {
        SEL selector = @selector(checkSpellingOfString:startingAt:language:wrap:inSpellDocumentWithTag:wordCount:);
        Method method = class_getInstanceMethod([NSSpellChecker class], selector);
        if (!method) abort();
        original = (void *)method_setImplementation(method, (IMP)observed);
        fputs("ROCKET_SPELL_OBSERVER_READY\n", stderr);
    }
}
