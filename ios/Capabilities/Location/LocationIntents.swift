import AppIntents

struct CurrentLocationIntent: AppIntent {
    static let title: LocalizedStringResource = "Get Current Location"
    static let description = IntentDescription("Read the current location.")
    static let openAppWhenRun = false

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.location.current"))
    }
}

struct AddGeofenceIntent: AppIntent {
    static let title: LocalizedStringResource = "Add Geofence"
    static let description = IntentDescription("Add a location boundary.")
    static let openAppWhenRun = false

    @Parameter(title: "Arguments JSON")
    var arguments: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.geofence.add", arguments: arguments))
    }
}

struct RemoveGeofenceIntent: AppIntent {
    static let title: LocalizedStringResource = "Remove Geofence"
    static let description = IntentDescription("Remove the selected location boundary.")
    static let openAppWhenRun = false

    @Parameter(title: "Arguments JSON")
    var arguments: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.geofence.remove", arguments: arguments))
    }
}

struct ListGeofencesIntent: AppIntent {
    static let title: LocalizedStringResource = "List Geofences"
    static let description = IntentDescription("List the saved location boundaries.")
    static let openAppWhenRun = false

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.geofence.list"))
    }
}

struct SignificantLocationIntent: AppIntent {
    static let title: LocalizedStringResource = "Set Significant Location Monitoring"
    static let description = IntentDescription("Read significant location changes.")
    static let openAppWhenRun = false

    @Parameter(title: "Arguments JSON")
    var arguments: String

    func perform() async throws -> some IntentResult & ReturnsValue<String> {
        .result(value: try await CommandIntent.run("iphone.location.significant", arguments: arguments))
    }
}

