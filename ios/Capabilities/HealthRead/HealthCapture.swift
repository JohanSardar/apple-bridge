import Foundation

struct HealthSampleRecord: Codable, Sendable {
    var id: String
    var type: String
    var start: String
    var end: String
    var value: Double?
    var unit: String?
    var category: Int?
    var details: [String: HealthValue]?
}

indirect enum HealthValue: Codable, Sendable {
    case text(String), number(Double), bool(Bool), array([HealthValue]), object([String: HealthValue])

    init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if let v = try? c.decode(String.self) { self = .text(v) }
        else if let v = try? c.decode(Bool.self) { self = .bool(v) }
        else if let v = try? c.decode(Double.self) { self = .number(v) }
        else if let v = try? c.decode([HealthValue].self) { self = .array(v) }
        else { self = .object(try c.decode([String: HealthValue].self)) }
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .text(let v): try c.encode(v)
        case .number(let v): try c.encode(v)
        case .bool(let v): try c.encode(v)
        case .array(let v): try c.encode(v)
        case .object(let v): try c.encode(v)
        }
    }
}

struct HealthPage: Sendable {
    let samples: [HealthSampleRecord]
    let deleted: [String]
    let anchor: Data
    let more: Bool
}

@MainActor
enum HealthReadAuthorization {
    static func request<T: Sendable>(_ types: [T], indirect: (T) -> Bool = { _ in false }, perObject: (T) -> Bool, bulk: ([T]) async throws -> Void, individual: (T) async throws -> Void) async throws {
        let requested = types.filter { !indirect($0) }
        let shared = requested.filter { !perObject($0) }
        if !shared.isEmpty { try await bulk(shared) }
        for type in requested where perObject(type) { try await individual(type) }
    }
}

@MainActor
protocol HealthReading {
    var streams: [String] { get }
    func page(stream: String, anchor: Data?) async throws -> HealthPage
}

struct HealthUpload: Codable, Sendable {
    let stream: String
    let sequence: UInt64
    let samples: [HealthSampleRecord]
    let deleted: [String]
}

private struct HealthCheckpoint: Codable {
    var sequence: UInt64 = 0
    var anchor: Data?
    var pending: HealthUpload?
    var pendingAnchor: Data?
}

@MainActor
final class HealthCapture {
    private let store: any HealthReading
    private let url: URL
    private var running = false
    private var checkpoints: [String: HealthCheckpoint]

    init(store: any HealthReading, url: URL) throws {
        self.store = store
        self.url = url
        checkpoints = FileManager.default.fileExists(atPath: url.path) ? try JSONDecoder().decode([String: HealthCheckpoint].self, from: Data(contentsOf: url)) : [:]
    }

    private func save() throws {
        try JSONEncoder().encode(checkpoints).write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
    }

    func capture(upload: (HealthUpload) async throws -> Void) async throws {
        guard !running else { return }
        running = true
        defer { running = false }
        for stream in store.streams {
            var checkpoint = checkpoints[stream] ?? HealthCheckpoint()
            if let pending = checkpoint.pending {
                try await upload(pending)
                checkpoint.sequence = pending.sequence
                checkpoint.anchor = checkpoint.pendingAnchor
                checkpoint.pending = nil
                checkpoint.pendingAnchor = nil
                checkpoints[stream] = checkpoint
                try save()
            }
            var more = true
            while more {
                let page = try await store.page(stream: stream, anchor: checkpoint.anchor)
                more = page.more
                if page.samples.isEmpty && page.deleted.isEmpty {
                    checkpoint.anchor = page.anchor
                    checkpoints[stream] = checkpoint
                    try save()
                    continue
                }
                let batch = HealthUpload(stream: stream, sequence: checkpoint.sequence + 1, samples: page.samples, deleted: page.deleted)
                checkpoint.pending = batch
                checkpoint.pendingAnchor = page.anchor
                checkpoints[stream] = checkpoint
                try save()
                try await upload(batch)
                checkpoint.sequence = batch.sequence
                checkpoint.anchor = page.anchor
                checkpoint.pending = nil
                checkpoint.pendingAnchor = nil
                checkpoints[stream] = checkpoint
                try save()
            }
        }
    }
}
