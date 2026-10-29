@MainActor
extension SessionSources {
 static func registerMapsSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
 try? await peer.register(MapsCommands())
 try? await peer.register(LocationStatus())
 try? await peer.registerBlobReader("maps-snapshot:", source: "maps") { handle, offset, length in try await MapsQueries.readBlob(handle, offset: offset, length: length) }
 try? await peer.registerQueryOutput("maps.snapshot") { args in try await MapsQueries.snapshot(args) }
 try? await peer.registerQuery("location.current") { args in try await MapsQueries.run("location.current", args: args) }
 try? await peer.registerQuery("location.geocode") { args in try await MapsQueries.run("location.geocode", args: args) }
 try? await peer.registerQuery("location.reverse_geocode") { args in try await MapsQueries.run("location.reverse_geocode", args: args) }
 try? await peer.registerQuery("maps.search") { args in try await MapsQueries.run("maps.search", args: args) }
 try? await peer.registerQuery("maps.nearby") { args in try await MapsQueries.run("maps.nearby", args: args) }
 try? await peer.registerQuery("maps.directions") { args in try await MapsQueries.run("maps.directions", args: args) }
 try? await peer.registerQuery("maps.eta") { args in try await MapsQueries.run("maps.eta", args: args) }
 }
}
