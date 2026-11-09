#import <AppKit/AppKit.h>
#include <stdint.h>
typedef struct { uint32_t id, group, parent; const char *title, *key; uint32_t modifiers; bool enabled, checked; } RMItem;
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
static NSMutableDictionary<NSNumber *, NSMenuItem *> *rows;
static NSArray<NSArray<NSNumber *> *> *structure;
int rocket_menu_sync(const RMItem *items, size_t count, void (*repaint)(void)) {
    if (![NSThread isMainThread] || !NSApp || count > 256) return -1;
    @autoreleasepool { @try {
        wake = repaint;
        if (!pending) pending = [NSMutableArray new];
        if (!target) target = [RocketMenuTarget new];
        // Update existing rows while a menu is open; replacing the menu during a
        // background refresh can discard the user's click or keyboard selection.
        NSMutableArray *nextStructure = [NSMutableArray new];
        for (size_t i = 0; i < count; i++) [nextStructure addObject:@[@(items[i].id), @(items[i].group), @(items[i].parent)]];
        bool same = installed && NSApp.mainMenu == installed && [structure isEqualToArray:nextStructure];
        if (same) {
            for (size_t i = 0; i < count; i++) {
                const RMItem *item = items + i;
                NSMenuItem *entry = rows[@(item->id)];
                entry.title = [NSString stringWithUTF8String:item->title]; entry.keyEquivalent = [NSString stringWithUTF8String:item->key];
                entry.enabled = item->enabled; entry.state = item->checked ? NSControlStateValueOn : NSControlStateValueOff;
                entry.keyEquivalentModifierMask = ((item->modifiers & 1) ? NSEventModifierFlagControl : 0) | ((item->modifiers & 2) ? NSEventModifierFlagOption : 0) | ((item->modifiers & 4) ? NSEventModifierFlagShift : 0) | ((item->modifiers & 8) ? NSEventModifierFlagCommand : 0);
            }
            return 0;
        }
        NSMenu *bar = [NSMenu new];
        NSMutableDictionary<NSNumber *, NSMenuItem *> *nextRows = [NSMutableDictionary new];
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
            NSMenu *destination = menus[item->group];
            if (item->parent) {
                NSMenuItem *parent = nextRows[@(item->parent)];
                if (!parent) return -1;
                if (!parent.submenu) {
                    parent.submenu = [[NSMenu alloc] initWithTitle:parent.title];
                    parent.submenu.autoenablesItems = NO;
                    parent.action = nil;
                }
                destination = parent.submenu;
            }
            [destination addItem:entry]; nextRows[@(item->id)] = entry;
        }
        NSMenu *app = menus[0]; [app addItem:NSMenuItem.separatorItem];
        NSMenuItem *hide = [[NSMenuItem alloc] initWithTitle:@"Hide Rocket" action:@selector(hide:) keyEquivalent:@"h"];
        hide.target = NSApp; [app addItem:hide];
        NSMenuItem *services = [[NSMenuItem alloc] initWithTitle:@"Services" action:nil keyEquivalent:@""];
        services.submenu = [NSMenu new]; [app addItem:services]; NSApp.servicesMenu = services.submenu;
        NSMenuItem *minimize = [[NSMenuItem alloc] initWithTitle:@"Minimize" action:@selector(performMiniaturize:) keyEquivalent:@"m"];
        [menus[6] addItem:minimize];
        // A command queued from a replaced structure must not target a new row.
        [pending removeAllObjects]; rows = nextRows; structure = nextStructure;
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
    if (![NSThread isMainThread] || NSApp.mainMenu != installed || pending.count >= 32) return false;
    NSMenuItem *item = rows[@(id)];
    if (!item || item.target != target || item.submenu || !item.enabled) return false;
    // Cocoa disables the entire submenu when any parent is disabled.
    for (NSMenu *menu = item.menu; menu.supermenu; menu = menu.supermenu) {
        NSInteger index = [menu.supermenu indexOfItemWithSubmenu:menu];
        if (index < 0 || ![menu.supermenu itemAtIndex:index].enabled) return false;
    }
    [target invoke:item]; return true;
}
