#if LAB
import ApplicationServices

@MainActor
enum RemindersFixtureChecks {
    static func run() {
        let title = "[ab-test 11111111-2222-3333-4444-555555555555]"
        let parent = title + " parent"
        precondition(RemindersFixtureUI.matches([parent], name: parent, title: title))
        precondition(RemindersFixtureUI.matches([parent + " #abtest"], name: parent, title: title))
        precondition(RemindersFixtureUI.matches([parent + ", flagged"], name: parent, title: title))
        precondition(RemindersFixtureUI.matches([parent + "\nnotes"], name: parent, title: title))
        precondition(!RemindersFixtureUI.matches([parent + " other"], name: parent, title: title))
        precondition(!RemindersFixtureUI.matches(["Personal #abtest"], name: parent, title: title))
        precondition(!RemindersFixtureUI.matches(["New Section #abtest"], name: "New Section", title: title))
        precondition(RemindersFixtureUI.editableRole(kAXTextFieldRole))
        precondition(RemindersFixtureUI.editableRole(kAXTextAreaRole))
        precondition(!RemindersFixtureUI.editableRole(kAXMenuItemRole))
        precondition(!RemindersFixtureUI.editableRole(kAXRowRole))
        precondition(RemindersFixtureUI.editorRole(kAXWindowRole))
        precondition(RemindersFixtureUI.editorRole(kAXSheetRole))
        precondition(!RemindersFixtureUI.editorRole(kAXTableRole))
        precondition(RemindersFixtureUI.scopedActionRole(kAXButtonRole))
        precondition(RemindersFixtureUI.scopedActionRole(kAXPopUpButtonRole))
        precondition(!RemindersFixtureUI.scopedActionRole(kAXMenuItemRole))
        precondition(!RemindersFixtureUI.scopedActionRole(kAXMenuBarItemRole))
        precondition(RemindersFixtureUI.actionMatches("AXMoveToTop", description: "", name: "Move To Top"))
        precondition(RemindersFixtureUI.actionMatches("custom", description: "Move To Top", name: "Move To Top"))
        precondition(!RemindersFixtureUI.actionMatches("AXMoveToBottom", description: "Move To Bottom", name: "Move To Top"))
        precondition(!RemindersFixtureUI.actionMatches("AXPress", description: "Press", name: "Move To Top"))
        precondition(RemindersFixtureUI.smartListDeleteConfirmation(["Delete Smart List “" + title + " smart”?"], title: title + " smart"))
        precondition(!RemindersFixtureUI.smartListDeleteConfirmation(["Delete Smart List “Unrelated List”?"], title: title + " smart"))
        precondition(!RemindersFixtureUI.smartListDeleteConfirmation(["Delete Reminder “" + title + " smart”?"], title: title + " smart"))
        precondition(RemindersFixtureUI.matches(["Title", "Value", "Description"], name: "", title: title))
        precondition(RemindersFixtureUI.ownsReminderPopover([parent], title: title))
        precondition(RemindersFixtureUI.ownsReminderPopover([title + " child"], title: title))
        precondition(!RemindersFixtureUI.ownsReminderPopover(["[ab-test another-run] child"], title: title))
        precondition(!RemindersFixtureUI.ownsReminderPopover([title], title: title))
        print("reminders fixture checks passed")
    }
}
#endif
