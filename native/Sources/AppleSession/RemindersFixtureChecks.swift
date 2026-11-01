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
        print("reminders fixture checks passed")
    }
}
#endif
