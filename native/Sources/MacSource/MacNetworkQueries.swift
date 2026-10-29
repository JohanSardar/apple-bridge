import Foundation
import Network
import Darwin
import SourceSupport

public enum MacNetworkQueries {
    public static func check(_ args: JSON) async throws -> JSON {
        guard args == .object([:]) else { throw SourceFailure.invalid }
        let monitor = NWPathMonitor()
        let (paths, continuation) = AsyncStream<Bool>.makeStream(bufferingPolicy: .bufferingNewest(1))
        monitor.pathUpdateHandler = { path in continuation.yield(path.status == .satisfied); continuation.finish() }
        monitor.start(queue: DispatchQueue(label: "mac.network.check"))
        var connected = false
        for await status in paths { connected = status; break }
        monitor.cancel()
        let dns = await Task.detached {
            var result: UnsafeMutablePointer<addrinfo>?
            let status = getaddrinfo("apple.com", nil, nil, &result)
            if let result { freeaddrinfo(result) }
            return status == 0
        }.value
        var interfaces: [JSON] = []
        var head: UnsafeMutablePointer<ifaddrs>?
        if getifaddrs(&head) == 0 {
            defer { freeifaddrs(head) }
            var next = head
            while let item = next {
                defer { next = item.pointee.ifa_next }
                guard item.pointee.ifa_addr?.pointee.sa_family == UInt8(AF_LINK), let data = item.pointee.ifa_data?.assumingMemoryBound(to: if_data.self).pointee else { continue }
                interfaces.append(.object(["name": .string(String(cString: item.pointee.ifa_name)), "received_bytes": .number(Double(data.ifi_ibytes)), "sent_bytes": .number(Double(data.ifi_obytes))]))
            }
        }
        return .object(["connected": .bool(connected), "dns": .bool(dns), "ping": .string("limited: ICMP unavailable"), "interfaces": .array(interfaces)])
    }
}
