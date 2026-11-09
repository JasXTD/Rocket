#import <AppKit/AppKit.h>
#include <stdint.h>
typedef struct { uint32_t id, group; const char *title, *key; uint32_t modifiers; bool enabled, checked; } RMItem;
static NSMutableArray<NSNumber *> *pending;
static void (*wake)(void);
@interface RocketMenuTarget : NSObject
- (void)invoke:(NSMenuItem *)sender;
@end
@implementation RocketMenuTarget
- (void)invoke:(NSMenuItem *)sender {
    if (sender.enabled && pending.count < 32) { [pending addObject:@(sender.tag)]; if (wake) wake(); }
}
@end
static RocketMenuTarget *target;
static NSMenu *installed;
int rocket_menu_sync(const RMItem *items, size_t count, void (*repaint)(void)) {
    if (![NSThread isMainThread] || !NSApp || count > 256) return -1;
    @autoreleasepool { @try {
        wake = repaint;
        if (!pending) pending = [NSMutableArray new];
        if (!target) target = [RocketMenuTarget new];
        // Update existing rows while a menu is open; replacing the menu during a
        // background refresh can discard the user's click or keyboard selection.
        bool same = installed && NSApp.mainMenu == installed;
        if (same) for (size_t i = 0; i < count; i++) {
            if (items[i].group >= installed.numberOfItems || ![[installed itemAtIndex:items[i].group].submenu itemWithTag:items[i].id]) { same = false; break; }
        }
        if (same) {
            for (size_t i = 0; i < count; i++) {
                const RMItem *item = items + i;
                NSMenuItem *entry = [[installed itemAtIndex:item->group].submenu itemWithTag:item->id];
                entry.title = [NSString stringWithUTF8String:item->title]; entry.keyEquivalent = [NSString stringWithUTF8String:item->key];
                entry.enabled = item->enabled; entry.state = item->checked ? NSControlStateValueOn : NSControlStateValueOff;
                entry.keyEquivalentModifierMask = ((item->modifiers & 1) ? NSEventModifierFlagControl : 0) | ((item->modifiers & 2) ? NSEventModifierFlagOption : 0) | ((item->modifiers & 4) ? NSEventModifierFlagShift : 0) | ((item->modifiers & 8) ? NSEventModifierFlagCommand : 0);
            }
            return 0;
        }
        NSMenu *bar = [NSMenu new];
        NSArray<NSString *> *titles = @[@"Rocket", @"File", @"Edit", @"View", @"Repository", @"Terminal", @"Window", @"Help"];
        NSMutableArray<NSMenu *> *menus = [NSMutableArray new];
        for (NSString *title in titles) {
            NSMenu *menu = [[NSMenu alloc] initWithTitle:title]; menu.autoenablesItems = NO;
            NSMenuItem *top = [[NSMenuItem alloc] initWithTitle:title action:nil keyEquivalent:@""];
            top.submenu = menu; [bar addItem:top]; [menus addObject:menu];
        }
        for (size_t i = 0; i < count; i++) {
            const RMItem *item = items + i; if (item->group >= menus.count) return -1;
            NSString *title = [NSString stringWithUTF8String:item->title], *key = [NSString stringWithUTF8String:item->key];
            if (!title || !key) return -1;
            NSMenuItem *entry = [[NSMenuItem alloc] initWithTitle:title action:@selector(invoke:) keyEquivalent:key];
            entry.target = target; entry.tag = item->id; entry.enabled = item->enabled;
            entry.state = item->checked ? NSControlStateValueOn : NSControlStateValueOff;
            entry.keyEquivalentModifierMask = ((item->modifiers & 1) ? NSEventModifierFlagControl : 0)
                | ((item->modifiers & 2) ? NSEventModifierFlagOption : 0)
                | ((item->modifiers & 4) ? NSEventModifierFlagShift : 0)
                | ((item->modifiers & 8) ? NSEventModifierFlagCommand : 0);
            [menus[item->group] addItem:entry];
        }
        NSMenu *app = menus[0]; [app addItem:NSMenuItem.separatorItem];
        NSMenuItem *hide = [[NSMenuItem alloc] initWithTitle:@"Hide Rocket" action:@selector(hide:) keyEquivalent:@"h"];
        hide.target = NSApp; [app addItem:hide];
        NSMenuItem *services = [[NSMenuItem alloc] initWithTitle:@"Services" action:nil keyEquivalent:@""];
        services.submenu = [NSMenu new]; [app addItem:services]; NSApp.servicesMenu = services.submenu;
        NSMenuItem *minimize = [[NSMenuItem alloc] initWithTitle:@"Minimize" action:@selector(performMiniaturize:) keyEquivalent:@"m"];
        [menus[6] addItem:minimize];
        NSApp.windowsMenu = menus[6]; NSApp.helpMenu = menus[7]; NSApp.mainMenu = bar; installed = bar;
        return 0;
    } @catch (NSException *exception) { (void)exception; return -1; } }
}
uint32_t rocket_menu_pop(void) {
    if (![NSThread isMainThread] || !pending.count) return 0;
    uint32_t item = pending[0].unsignedIntValue; [pending removeObjectAtIndex:0]; return item;
}
// Read-only inspection and native invocation used by the first-party focused probe.
bool rocket_menu_invoke(uint32_t id) {
    if (![NSThread isMainThread]) return false;
    for (NSMenuItem *top in NSApp.mainMenu.itemArray) for (NSMenuItem *item in top.submenu.itemArray) {
        if (item.tag == id && item.target == target && item.enabled) { [target invoke:item]; return true; }
    }
    return false;
}
