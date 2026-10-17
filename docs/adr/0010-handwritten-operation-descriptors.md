# Handwritten operation descriptors generate REST, MCP and OpenAPI

Each operation is one handwritten descriptor (name, version, kind, source, grant, capability, input/output types, description, examples) from which Frontdoor registers REST routes, MCP tools and OpenAPI. JSON-Schema-first code generation was rejected: schemas cannot express authorization, conflicts or useful tool descriptions, and generation would precede one proven path.
