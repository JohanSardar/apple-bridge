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
        precondition(!RemindersFixtureUI.smartListDeleteConfirmation(["Delete Smart List “Pack”?"], title: title + " smart"))
        precondition(!RemindersFixtureUI.smartListDeleteConfirmation(["Delete Reminder “" + title + " smart”?"], title: title + " smart"))
        precondition(RemindersFixtureUI.matches(["Title", "Value", "Description"], name: "", title: title))
        let row = CGRect(x: 0, y: 0, width: 200, height: 32)
        let field = CGRect(x: 45, y: 6, width: 110, height: 18)
        let buttons = [CGRect(x: 4, y: 6, width: 24, height: 20), CGRect(x: 176, y: 6, width: 20, height: 20)]
        let background = RemindersFixtureUI.reminderBackground(row: row, title: field, buttons: buttons)
        precondition(background.map { point in row.contains(point) && !field.contains(point) && buttons.allSatisfy { !$0.contains(point) } } == true)
        precondition(RemindersFixtureUI.reminderBackground(row: row, title: row, buttons: []) == nil)
        precondition(RemindersFixtureUI.reminderBackground(row: row, title: field, buttons: [row]) == nil)
        print("reminders fixture checks passed")
    }
}
#endif
