// Disposable CI diagnostic: fixed synthetic strings only; never learn words.
#import <AppKit/AppKit.h>
#include <stdint.h>
#include <stdio.h>
typedef struct { size_t start, length; } RSRange;
extern void *rocket_spell_create(const uint8_t *, size_t, char *, size_t);
extern int rocket_spell_check(void *, const uint8_t *, size_t, RSRange *, size_t);
extern void rocket_spell_destroy(void *);
int main(void) {
    @autoreleasepool {
        [NSApplication sharedApplication];
        NSSpellChecker *checker = [NSSpellChecker sharedSpellChecker];
        NSMutableArray *results = [NSMutableArray array];
        for (NSString *language in @[checker.language.length ? checker.language : @"en", @"en", @"en_US"]) {
            for (NSString *text in @[@"Ths sentnce has misspelled wrds", @"hello world", @"資料 🚀 café mispellling"]) {
                NSInteger words = -2;
                NSInteger tag = [NSSpellChecker uniqueSpellDocumentTag];
                NSRange range = [checker checkSpellingOfString:text startingAt:0 language:language wrap:NO inSpellDocumentWithTag:tag wordCount:&words];
                [checker closeSpellDocumentWithTag:tag];
                [results addObject:@{@"language":language, @"text":text, @"words":@(words), @"found":@(range.location != NSNotFound), @"length":@(range.length)}];
            }
        }
        NSMutableArray *bridge = [NSMutableArray array];
        for (NSString *requested in @[@"", @"en-US"]) {
            NSData *request = [requested dataUsingEncoding:NSUTF8StringEncoding];
            char selected[128] = {0};
            void *handle = rocket_spell_create(request.bytes, request.length, selected, sizeof selected);
            NSMutableDictionary *result = [@{@"requested":requested, @"created":@(handle != NULL)} mutableCopy];
            if (handle) {
                NSData *text = [@"Ths sentnce has misspelled wrds" dataUsingEncoding:NSUTF8StringEncoding];
                RSRange ranges[32];
                result[@"language"] = [NSString stringWithUTF8String:selected];
                result[@"misspellings"] = @(rocket_spell_check(handle, text.bytes, text.length, ranges, 32));
                rocket_spell_destroy(handle);
            }
            [bridge addObject:result];
        }
        NSDictionary *report = @{@"default_language":checker.language ?: @"", @"available":checker.availableLanguages ?: @[], @"automatic":@(checker.automaticallyIdentifiesLanguages), @"native":results, @"bridge":bridge};
        NSData *json = [NSJSONSerialization dataWithJSONObject:report options:NSJSONWritingPrettyPrinted error:nil];
        if (!json) return 1;
        fwrite(json.bytes, 1, json.length, stdout);
        puts("");
    }
    return 0;
}
