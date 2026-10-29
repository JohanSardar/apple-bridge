import Foundation
import FamilyControls
import ManagedSettings

enum ScreenTimeState {
    static let group = "group.com.johansardar.applebridge.ios.screentime"
    static var defaults: UserDefaults { UserDefaults(suiteName: group)! }
    static var manual: ManagedSettingsStore { ManagedSettingsStore(named: .init("bridge.manual")) }
    static var downtime: ManagedSettingsStore { ManagedSettingsStore(named: .init("bridge.downtime")) }

    static var selection: FamilyActivitySelection {
        get {
            guard let data = defaults.data(forKey: "selection"),
                  let value = try? JSONDecoder().decode(FamilyActivitySelection.self, from: data) else { return FamilyActivitySelection() }
            return value
        }
        set { defaults.set(try? JSONEncoder().encode(newValue), forKey: "selection") }
    }

    static func apply(_ store: ManagedSettingsStore) {
        let value = selection
        BridgeShieldPolicy.apply(ManagedShieldAdapter(store: store), apps: value.applicationTokens, categories: value.categoryTokens, domains: value.webDomainTokens)
    }
}

struct ManagedShieldAdapter: BridgeShieldAdapter {
    let store: ManagedSettingsStore
    func set(apps: Set<ApplicationToken>?, categories: Set<ActivityCategoryToken>?, domains: Set<WebDomainToken>?) {
        store.shield.applications = apps
        store.shield.applicationCategories = categories.map { .specific($0) }
        store.shield.webDomains = domains
        store.shield.webDomainCategories = categories.map { .specific($0) }
    }
}
