// Disposable GitHub runner only. Observe service-disconnect behavior without
// changing production spelling results, dictionary contents or personal machines.
#import <AppKit/AppKit.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
typedef struct { size_t start, length; } RSRange;
extern void *rocket_spell_create(const uint8_t *, size_t, char *, size_t);
extern int rocket_spell_check(void *, const uint8_t *, size_t, RSRange *, size_t);
extern void rocket_spell_destroy(void *);
int main(void) {
    if (!getenv("GITHUB_ACTIONS") || strcmp(getenv("GITHUB_ACTIONS"),"true") ||
        !getenv("ROCKET_MACOS_SANDBOX_DISPOSABLE_USER") || strcmp(getenv("ROCKET_MACOS_SANDBOX_DISPOSABLE_USER"),"1")) return 2;
    @autoreleasepool {
        [NSApplication sharedApplication];
        NSSpellChecker *checker=[NSSpellChecker sharedSpellChecker];
        NSData *text=[@"Ths sentnce has misspelled wrds" dataUsingEncoding:NSUTF8StringEncoding];
        char language[128]={0};
        void *handle=rocket_spell_create((const uint8_t *)"en-US",5,language,sizeof language);
        if (!handle) return 3;
        RSRange ranges[32];
        NSMutableArray *rows=[NSMutableArray array];
        for (int iteration=0;iteration<12;iteration++) {
            int before=rocket_spell_check(handle,text.bytes,text.length,ranges,32);
            NSTask *stop=[[NSTask alloc] init];
            stop.executableURL=[NSURL fileURLWithPath:@"/usr/bin/killall"];
            // Exact process name, current disposable account only. No sudo.
            stop.arguments=@[@"-u",NSUserName(),@"AppleSpell"];
            stop.standardOutput=[NSFileHandle fileHandleWithNullDevice];
            stop.standardError=[NSFileHandle fileHandleWithNullDevice];
            NSError *error=nil;
            if (![stop launchAndReturnError:&error]) return 4;
            [stop waitUntilExit];
            int result=-99;NSInteger words=-2;
            NSRange native=NSMakeRange(NSNotFound,0);
            NSInteger tag=[NSSpellChecker uniqueSpellDocumentTag];
            double begin=CFAbsoluteTimeGetCurrent();
            if (iteration%2==0) result=rocket_spell_check(handle,text.bytes,text.length,ranges,32);
            native=[checker checkSpellingOfString:@"Ths sentnce has misspelled wrds" startingAt:0 language:@"en" wrap:NO inSpellDocumentWithTag:tag wordCount:&words];
            if (iteration%2) result=rocket_spell_check(handle,text.bytes,text.length,ranges,32);
            [checker closeSpellDocumentWithTag:tag];
            [rows addObject:@{@"iteration":@(iteration),@"before":@(before),@"stop_exit":@(stop.terminationStatus),@"bridge_first":@(iteration%2==0),@"bridge_result":@(result),@"native_found":@(native.location!=NSNotFound),@"native_length":@(native.length),@"word_count":@(words),@"elapsed_ms":@((CFAbsoluteTimeGetCurrent()-begin)*1000)}];
        }
        rocket_spell_destroy(handle);
        NSData *json=[NSJSONSerialization dataWithJSONObject:@{@"service_restart":rows} options:NSJSONWritingPrettyPrinted error:nil];
        if(!json)return 5;fwrite(json.bytes,1,json.length,stdout);puts("");
    }
    return 0;
}
