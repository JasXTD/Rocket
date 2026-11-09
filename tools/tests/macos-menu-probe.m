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
    RMItem rows[] = {{1,1,0,"Open 資料","o",8,true,false},{2,4,0,"Push","",0,false,false},{3,3,0,"Sidebar","\357\234\213",8,true,true},{4,7,0,"Check for Updates","",0,true,false},{5,4,0,"Branch","",0,true,false},{6,4,5,"Merge","",0,true,false}};
    assert(rocket_menu_sync(rows,6,repaint)==0);
    assert(NSApp.mainMenu.numberOfItems==8);
    assert([NSApp.mainMenu itemAtIndex:1].submenu.itemArray[0].keyEquivalentModifierMask==NSEventModifierFlagCommand);
    assert([NSApp.mainMenu itemAtIndex:3].submenu.itemArray[0].state==NSControlStateValueOn);
    assert(rocket_menu_invoke(1));assert(!rocket_menu_invoke(2));assert(rocket_menu_pop()==1);assert(rocket_menu_pop()==0);assert(repaints==1);
    NSMenu *original = NSApp.mainMenu;
    rows[1].enabled=true;rows[0].modifiers=8|4;
    assert(rocket_menu_sync(rows,6,repaint)==0);assert(rocket_menu_invoke(2));assert(rocket_menu_pop()==2);
    assert(NSApp.mainMenu==original);
    assert([NSApp.mainMenu itemAtIndex:3].submenu.itemArray[0].keyEquivalent.length==1);
    assert(rocket_menu_invoke(6));assert(rocket_menu_pop()==6);assert(!rocket_menu_invoke(5));
    rows[4].enabled=false;assert(rocket_menu_sync(rows,6,repaint)==0);assert(!rocket_menu_invoke(6));
    rows[4].enabled=true;assert(rocket_menu_sync(rows,6,repaint)==0);
    for(int i=0;i<32;i++)assert(rocket_menu_invoke(1));assert(!rocket_menu_invoke(1));
    for(int i=0;i<32;i++)assert(rocket_menu_pop()==1);assert(rocket_menu_pop()==0);
    assert(rocket_menu_sync(rows,4,repaint)==0);assert(!rocket_menu_invoke(6));
    puts("PASS native menus: Unicode, function keys, nested disabled parents, bounded queue, removed rows and in-place refresh");
} }
