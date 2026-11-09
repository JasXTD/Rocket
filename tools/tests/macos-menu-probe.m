#import <AppKit/AppKit.h>
#include <assert.h>
#include <sys/event.h>
#include <stddef.h>
// Compile the actual adapter, not a duplicate implementation.
#include "../../crates/rocket-git/src/macos_menu.m"
static int repaints;
static void repaint(void) { repaints++; }
int main(void) { @autoreleasepool {
    _Static_assert(sizeof(struct kevent64_s) == 48, "Rust event ABI");
    _Static_assert(offsetof(struct kevent64_s, data) == 16, "Rust event ABI");
    [NSApplication sharedApplication];
    RMItem rows[] = {{1,1,"Open 資料","o",8,true,false},{2,4,"Push","",0,false,false},{3,3,"Sidebar","b",8,true,true},{4,7,"Check for Updates","",0,true,false}};
    assert(rocket_menu_sync(rows,4,repaint)==0);
    assert(NSApp.mainMenu.numberOfItems==8);
    assert([NSApp.mainMenu itemAtIndex:1].submenu.itemArray[0].keyEquivalentModifierMask==NSEventModifierFlagCommand);
    assert([NSApp.mainMenu itemAtIndex:3].submenu.itemArray[0].state==NSControlStateValueOn);
    assert(rocket_menu_invoke(1));assert(!rocket_menu_invoke(2));assert(rocket_menu_pop()==1);assert(rocket_menu_pop()==0);assert(repaints==1);
    NSMenu *original = NSApp.mainMenu;
    rows[1].enabled=true;rows[0].modifiers=8|4;
    assert(rocket_menu_sync(rows,4,repaint)==0);assert(rocket_menu_invoke(2));assert(rocket_menu_pop()==2);
    assert(NSApp.mainMenu==original);
    puts("PASS native menus: eight groups, Unicode, modifiers, checks, disabled action, single dispatch and state refresh");
} }
