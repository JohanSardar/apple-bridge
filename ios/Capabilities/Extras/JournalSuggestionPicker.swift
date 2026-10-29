import SwiftUI
#if canImport(JournalingSuggestions)
import JournalingSuggestions
#endif

struct JournalSuggestionPicker: View {
    @AppStorage("capability.journal.enabled") private var enabled = false
    @State private var selected = false

    var body: some View {
        #if canImport(JournalingSuggestions)
        if ExtrasCapability.journalGranted && enabled {
            JournalingSuggestionsPicker("Choose a journal suggestion") { suggestion in
                ExtrasCapability.selectedSuggestion = suggestion.title
                selected = true
            }
            Text(selected ? "Suggestion ready for the gateway" : "Only a suggestion you choose is shared")
        }
        #else
        Text("Journal selection requires a physical iPhone")
        #endif
    }
}
