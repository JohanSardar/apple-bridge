import Foundation
import FoundationModels
import SourceSupport

public enum AIQueries {
    public static func run(_ operation: String, args: JSON) async throws -> JSON {
        guard Set(args.object.keys).isSubset(of: operation == "ai.local" ? ["prompt", "schema"] : ["text", "style"]) else { throw SourceFailure.invalid }
        let text = operation == "ai.local" ? args["prompt"].string : args["text"].string
        guard !text.isEmpty, text.utf8.count <= 16384, args["style"].string.utf8.count <= 1024 else { throw SourceFailure.invalid }
        var schema: GenerationSchema?
        if args["schema"] != .null {
            let value = args["schema"]
            guard value["type"].string == "object", Set(value.object.keys).isSubset(of: ["type", "properties", "required", "additionalProperties"]), !value["properties"].object.isEmpty, value["properties"].object.count <= 16 else { throw SourceFailure.invalid }
            guard value["additionalProperties"] == .null || value["additionalProperties"] == .bool(false) else { throw SourceFailure.invalid }
            if case .array = value["required"] {} else { throw SourceFailure.invalid }
            let required = Set(value["required"].array.map(\.string))
            guard required.isSubset(of: Set(value["properties"].object.keys)) else { throw SourceFailure.invalid }
            let properties = try value["properties"].object.keys.sorted().map { name -> DynamicGenerationSchema.Property in
                let property = value["properties"][name]
                guard !name.isEmpty, name.utf8.count <= 128, Set(property.object.keys).isSubset(of: ["type"]) else { throw SourceFailure.invalid }
                let type: DynamicGenerationSchema
                switch property["type"].string {
                case "string": type = DynamicGenerationSchema(type: String.self)
                case "integer": type = DynamicGenerationSchema(type: Int.self)
                case "number": type = DynamicGenerationSchema(type: Double.self)
                case "boolean": type = DynamicGenerationSchema(type: Bool.self)
                default: throw SourceFailure.invalid
                }
                return DynamicGenerationSchema.Property(name: name, schema: type, isOptional: !required.contains(name))
            }
            schema = try GenerationSchema(root: DynamicGenerationSchema(name: "Response", properties: properties), dependencies: [])
        }
        switch SystemLanguageModel.default.availability {
        case .available:
            let prompt = operation == "ai.local" ? text : "Rewrite the following text in this style: \(args["style"].string)\n\(text)"
            if let schema {
                let response = try await LanguageModelSession().respond(to: prompt, schema: schema)
                try Task.checkCancellation()
                return .object(["status": .string("available"), "text": .string(response.content.jsonString)])
            }
            let response = try await LanguageModelSession().respond(to: prompt)
            try Task.checkCancellation()
            return .object(["status": .string("available"), "text": .string(response.content)])
        case .unavailable(let reason): return unavailable(String(describing: reason))
        }
    }
    private static func unavailable(_ reason: String) -> JSON {
        .object(["status": .string("unavailable"), "reason": .string(reason)])
    }
}
