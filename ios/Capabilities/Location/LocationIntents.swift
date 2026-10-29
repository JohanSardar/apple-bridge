import AppIntents

struct CurrentLocationIntent: AppIntent {
    static let title: LocalizedStringResource = "Get Current Location"
    static let description = IntentDescription("Run iphone.location.current on this iPhone.")
    static let openAppWhenRun = false

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.location.current"))
    }
}

struct AddGeofenceIntent: AppIntent {
    static let title: LocalizedStringResource = "Add Geofence"
    static let description = IntentDescription("Run iphone.geofence.add on this iPhone.")
    static let openAppWhenRun = false

    @Parameter(title: "Arguments JSON")
    var arguments: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.geofence.add", arguments: arguments))
    }
}

struct RemoveGeofenceIntent: AppIntent {
    static let title: LocalizedStringResource = "Remove Geofence"
    static let description = IntentDescription("Run iphone.geofence.remove on this iPhone.")
    static let openAppWhenRun = false

    @Parameter(title: "Arguments JSON")
    var arguments: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.geofence.remove", arguments: arguments))
    }
}

struct ListGeofencesIntent: AppIntent {
    static let title: LocalizedStringResource = "List Geofences"
    static let description = IntentDescription("Run iphone.geofence.list on this iPhone.")
    static let openAppWhenRun = false

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.geofence.list"))
    }
}

struct SignificantLocationIntent: AppIntent {
    static let title: LocalizedStringResource = "Set Significant Location Monitoring"
    static let description = IntentDescription("Run iphone.location.significant on this iPhone.")
    static let openAppWhenRun = false

    @Parameter(title: "Arguments JSON")
    var arguments: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.location.significant", arguments: arguments))
    }
}

