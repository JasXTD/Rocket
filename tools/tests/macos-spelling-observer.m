// By default this only observes the packaged app. One explicit CI mode models
// unavailable native replies for fixed synthetic inputs in a disposable process.
#import <AppKit/AppKit.h>
#import <objc/runtime.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
static NSRange (*original)(id, SEL, NSString *, NSInteger, NSString *, BOOL, NSInteger, NSInteger *);
static NSArray *(*originalFull)(id, SEL, NSString *, NSRange, NSTextCheckingTypes, NSDictionary *,
                                NSInteger, NSOrthography *__autoreleasing *, NSInteger *);
static BOOL inject, firstFailed, partialFailed, fieldFailed, failConfirmation;
static NSInteger lastTag;
static void record(NSDictionary *event) {
    NSData *json = [NSJSONSerialization dataWithJSONObject:event options:0 error:nil];
    if (json) { fputs("ROCKET_SPELL_TRACE ", stderr); fwrite(json.bytes, 1, json.length, stderr); fputc('\n', stderr); fflush(stderr); }
}
static NSRange observed(id self, SEL selector, NSString *text, NSInteger start,
                        NSString *language, BOOL wrap, NSInteger tag, NSInteger *words) {
    double begin = CFAbsoluteTimeGetCurrent();
    BOOL zeroed = inject && !start && [text isEqualToString:@"Anotherrr unverified field"];
    BOOL skip = zeroed;
    if (inject && !start && !firstFailed && [text isEqualToString:@"Ths sentnce has misspelled wrds"]) {
        skip = firstFailed = YES;
    } else if (inject && start > 0 && !partialFailed && [text isEqualToString:@"wrngg phrase with anothrrr typo"]) {
        skip = partialFailed = YES;
    } else if (inject && !start && !fieldFailed && [text isEqualToString:@"Mispellling this field"]) {
        skip = fieldFailed = failConfirmation = YES;
    }
    NSRange result;
    if (skip) {
        if (words) *words = zeroed ? 0 : -1;
        result = NSMakeRange(NSNotFound, 0);
    } else {
        result = original(self, selector, text, start, language, wrap, tag, words);
        if (inject && words && [text isEqualToString:@"hello world"]) *words = -1;
    }
    lastTag = tag;
    @autoreleasepool {
        NSDictionary *event = @{
            @"event":@"spell_native_return", @"text_units":@(text.length),
            @"at_ms":@([[NSDate date] timeIntervalSince1970] * 1000.0),
            @"start":@(start), @"language":language ?: @"",
            @"not_found":@(result.location == NSNotFound), @"length":@(result.length),
            @"location":@(result.location), @"wrap":@(wrap),
            @"word_count_requested":@(words != NULL), @"words":words ? @(*words) : NSNull.null,
            @"simulated_failure":@(skip),
            @"elapsed_ms":@((CFAbsoluteTimeGetCurrent() - begin) * 1000.0)
        };
        record(event);
    }
    return result;
}
static NSArray *observedFull(id self, SEL selector, NSString *text, NSRange range, NSTextCheckingTypes types,
                             NSDictionary *options, NSInteger tag, NSOrthography *__autoreleasing *orthography, NSInteger *words) {
    BOOL unavailable = inject && failConfirmation && [text isEqualToString:@"Mispellling this field"];
    BOOL zeroed = inject && [text isEqualToString:@"Anotherrr unverified field"];
    NSArray *results;
    if (unavailable || zeroed) {
        if (unavailable) failConfirmation = NO;
        results = zeroed ? @[] : nil;
        if (words) *words = zeroed ? 0 : -1;
        if (orthography) *orthography = nil;
    } else {
        results = unavailable ? nil : originalFull(self, selector, text, range, types, options, tag, orthography, words);
        if (inject && [text isEqualToString:@"hello world"] && results) {
            if (words) *words = -1;
            if (orthography) *orthography = options[NSTextCheckingOrthographyKey];
        }
    }
    if (inject) record(@{@"event":@"spell_confirmation", @"text_units":@(text.length), @"start":@(range.location),
        @"same_document":@(tag == lastTag), @"spelling_only":@(types == NSTextCheckingTypeSpelling),
        @"language": [options[NSTextCheckingOrthographyKey] dominantLanguage] ?: @"",
        @"answered":@(results != nil), @"matches":@(results.count),
        @"words": words ? @(*words) : NSNull.null, @"used_language":@(orthography && *orthography)});
    return results;
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
        const char *mode = getenv("ROCKET_SPELL_TEST_MODE");
        inject = mode && !strcmp(mode, "availability-fixtures");
        if (inject) {
            Method full = class_getInstanceMethod([NSSpellChecker class],
                @selector(checkString:range:types:options:inSpellDocumentWithTag:orthography:wordCount:));
            if (!full) abort();
            originalFull = (void *)method_setImplementation(full, (IMP)observedFull);
        }
        fputs("ROCKET_SPELL_OBSERVER_READY\n", stderr);
    }
}
