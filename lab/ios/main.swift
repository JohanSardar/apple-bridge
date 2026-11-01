import Foundation

precondition(!BridgeShieldPolicy.authorized(status: 0))
precondition(!BridgeShieldPolicy.authorized(status: 1))
precondition(BridgeShieldPolicy.authorized(status: 2))
precondition(BridgeShieldPolicy.authorized(status: 3))
precondition(!BridgeShieldPolicy.authorized(status: 4))

final class FakeManagedSettings: BridgeShieldAdapter {
    var apps: Set<String>?
    var categories: Set<String>?
    var domains: Set<String>?
    func set(apps: Set<String>?, categories: Set<String>?, domains: Set<String>?) {
        self.apps = apps
        self.categories = categories
        self.domains = domains
    }
}

let manual = FakeManagedSettings()
let downtime = FakeManagedSettings()
let apple = FakeManagedSettings()
apple.apps = ["system"]
BridgeShieldPolicy.apply(manual, apps: ["test-app"], categories: ["test-category"], domains: ["example.test"])
precondition(manual.apps?.count == 1 && manual.categories?.count == 1 && manual.domains?.count == 1)
BridgeShieldPolicy.apply(downtime, apps: ["scheduled"], categories: [], domains: [])
precondition(downtime.apps?.count == 1 && downtime.categories == nil && downtime.domains == nil)
BridgeShieldPolicy.apply(downtime, apps: [], categories: [], domains: [])
precondition(downtime.apps == nil && manual.apps?.count == 1)
BridgeShieldPolicy.apply(manual, apps: [], categories: [], domains: [])
precondition(manual.apps == nil && manual.categories == nil && manual.domains == nil)
precondition(apple.apps == ["system"])
precondition(BridgeShieldPolicy.validSchedule(start: 1380, end: 60))
precondition(!BridgeShieldPolicy.validSchedule(start: 60, end: 60))
precondition(!BridgeShieldPolicy.validSchedule(start: -1, end: 60))
precondition(!BridgeShieldPolicy.validSchedule(start: 60, end: 70))
print("shield_policy_pass")
