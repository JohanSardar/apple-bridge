import Foundation

protocol BridgeShieldAdapter {
    associatedtype App: Hashable
    associatedtype Category: Hashable
    associatedtype Domain: Hashable
    func set(apps: Set<App>?, categories: Set<Category>?, domains: Set<Domain>?)
}

enum BridgeShieldPolicy {
    static func authorized(status: Int) -> Bool { status == 2 || status == 3 }

    static func apply<A: BridgeShieldAdapter>(_ adapter: A, apps: Set<A.App>, categories: Set<A.Category>, domains: Set<A.Domain>) {
        adapter.set(apps: apps.isEmpty ? nil : apps, categories: categories.isEmpty ? nil : categories, domains: domains.isEmpty ? nil : domains)
    }

    static func validSchedule(start: Int, end: Int) -> Bool {
        (0..<1440).contains(start) && (0..<1440).contains(end) && (end - start + 1440) % 1440 >= 15
    }
}
