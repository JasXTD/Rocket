// Cocoa owns all spell objects on its main queue. Worker callers wait off the UI
// thread; no Objective-C exception may unwind through Rust.
#import <AppKit/AppKit.h>
#include <stdint.h>
typedef struct { size_t start, length; } RSRange;
@interface RocketSpellContext : NSObject
@property(strong) NSSpellChecker *checker;
@property(copy) NSString *language;
@property NSInteger tag;
@end
@implementation RocketSpellContext
@end
static void onMain(void (^work)(void)) {
    if ([NSThread isMainThread]) work(); else dispatch_sync(dispatch_get_main_queue(), work);
}
static NSString *string(const uint8_t *bytes, size_t length) {
    return [[NSString alloc] initWithBytes:bytes length:length encoding:NSUTF8StringEncoding];
}
void *rocket_spell_create(const uint8_t *requested, size_t length, char *language, size_t capacity) {
    __block void *result = NULL;
    onMain(^{ @autoreleasepool { @try {
        [NSApplication sharedApplication];
        NSSpellChecker *checker = [NSSpellChecker sharedSpellChecker];
        NSString *wanted = length ? [string(requested, length) stringByReplacingOccurrencesOfString:@"-" withString:@"_"] : checker.language;
        if ([wanted isEqualToString:@"en_US"]) wanted = @"en";
        if (![checker.availableLanguages containsObject:wanted]) {
            if (length) return;
            wanted = @"en";
        }
        if (!checker || ![checker.availableLanguages containsObject:wanted] || ![wanted getCString:language maxLength:capacity encoding:NSUTF8StringEncoding]) return;
        RocketSpellContext *context = [RocketSpellContext new];
        context.checker = checker; context.language = wanted; context.tag = [NSSpellChecker uniqueSpellDocumentTag];
        result = (__bridge_retained void *)context;
    } @catch (NSException *exception) { (void)exception; } } });
    return result;
}
int rocket_spell_check(void *handle, const uint8_t *bytes, size_t length, RSRange *ranges, size_t capacity) {
    __block int result = -1;
    onMain(^{ @autoreleasepool { @try {
        RocketSpellContext *context = (__bridge RocketSpellContext *)handle;
        NSString *text = string(bytes, length); if (!text) return;
        NSUInteger start = 0; size_t count = 0;
        while (start < text.length) {
            NSRange range = [context.checker checkSpellingOfString:text startingAt:start language:context.language wrap:NO inSpellDocumentWithTag:context.tag wordCount:NULL];
            if (!range.length || range.location == NSNotFound) break;
            if (range.location < start || NSMaxRange(range) > text.length || count == capacity) return;
            ranges[count++] = (RSRange){range.location, range.length}; start = NSMaxRange(range);
        }
        result = (int)count;
    } @catch (NSException *exception) { (void)exception; } } });
    return result;
}
int rocket_spell_suggest(void *handle, const uint8_t *bytes, size_t length, char *output, size_t slots, size_t stride) {
    __block int result = -1;
    onMain(^{ @autoreleasepool { @try {
        RocketSpellContext *context = (__bridge RocketSpellContext *)handle;
        NSString *word = string(bytes, length); if (!word) return;
        NSArray<NSString *> *guesses = [context.checker guessesForWordRange:NSMakeRange(0, word.length) inString:word language:context.language inSpellDocumentWithTag:context.tag];
        size_t count = 0;
        for (NSString *guess in guesses) {
            if (count == slots) break;
            if ([guess getCString:output + count * stride maxLength:stride encoding:NSUTF8StringEncoding]) count++;
        }
        result = (int)count;
    } @catch (NSException *exception) { (void)exception; } } });
    return result;
}
int rocket_spell_word(void *handle, const uint8_t *bytes, size_t length, int learn) {
    __block int result = -1;
    onMain(^{ @autoreleasepool { @try {
        RocketSpellContext *context = (__bridge RocketSpellContext *)handle;
        NSString *word = string(bytes, length); if (!word) return;
        if (learn) [context.checker learnWord:word];
        else [context.checker ignoreWord:word inSpellDocumentWithTag:context.tag];
        result = 0;
    } @catch (NSException *exception) { (void)exception; } } });
    return result;
}
void rocket_spell_destroy(void *handle) {
    onMain(^{ @autoreleasepool {
        RocketSpellContext *context = CFBridgingRelease(handle);
        @try { [context.checker closeSpellDocumentWithTag:context.tag]; }
        @catch (NSException *exception) { (void)exception; }
    } });
}
