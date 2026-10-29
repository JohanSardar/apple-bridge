import Foundation
import SourceSupport

private final class CatalogRedirects: NSObject, URLSessionTaskDelegate, Sendable {
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest, completionHandler: @escaping @Sendable (URLRequest?) -> Void) {
        completionHandler(nil)
    }
}

public enum MusicCatalog {
    public static func search(_ args: JSON) async throws -> JSON {
        let term = args["term"].string.trimmingCharacters(in: .whitespacesAndNewlines)
        let entity = args["entity"].string.isEmpty ? "song" : args["entity"].string
        let fields = args.object
        guard case .object = args, case .string = args["term"] else { throw SourceFailure.invalid }
        if let value = fields["entity"] { guard case .string = value else { throw SourceFailure.invalid } }
        let limitValue = fields["limit"] == nil ? 25 : args["limit"].number
        guard limitValue >= 1, limitValue <= 100, limitValue.rounded() == limitValue else { throw SourceFailure.invalid }
        let limit = Int(limitValue)
        guard Set(fields.keys).isSubset(of: ["term", "entity", "limit"]),
              !term.isEmpty, term.utf8.count <= 512,
              ["song", "album", "musicArtist"].contains(entity),
              limit >= 1, limit <= 100, fields["limit"] == nil || args["limit"] == .number(Double(limit)) else { throw SourceFailure.invalid }
        var url = URLComponents(string: "https://itunes.apple.com/search")!
        url.queryItems = [URLQueryItem(name: "term", value: term), URLQueryItem(name: "entity", value: entity), URLQueryItem(name: "media", value: "music"), URLQueryItem(name: "limit", value: String(limit))]
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 15
        config.timeoutIntervalForResource = 20
        config.httpCookieStorage = nil
        config.urlCache = nil
        let session = URLSession(configuration: config, delegate: CatalogRedirects(), delegateQueue: nil)
        defer { session.invalidateAndCancel() }
        let (bytes, response) = try await session.bytes(from: url.url!)
        guard let http = response as? HTTPURLResponse, http.statusCode == 200 else { throw SourceFailure.unavailable }
        var data = Data()
        for try await byte in bytes {
            try Task.checkCancellation()
            guard data.count < 1 << 20 else { throw SourceFailure.invalid }
            data.append(byte)
        }
        guard let root = try JSONSerialization.jsonObject(with: data) as? [String: Any], let results = root["results"] as? [[String: Any]] else { throw SourceFailure.corrupt }
        let items: [JSON] = results.prefix(limit).compactMap { row in
            let key = entity == "song" ? "trackId" : entity == "album" ? "collectionId" : "artistId"
            guard let id = row[key] as? NSNumber else { return nil }
            let name = row["trackName"] as? String ?? row["collectionName"] as? String ?? row["artistName"] as? String ?? ""
            return .object(["catalog_id": .string(id.stringValue), "name": .string(name), "artist": .string(row["artistName"] as? String ?? ""), "kind": .string(entity)])
        }
        return .object(["items": .array(items)])
    }
}
