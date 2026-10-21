import Foundation

public enum ThumbHash {
    public static func encode(width: Int, height: Int, rgba: [UInt8]) -> Data? {
        guard width > 0, height > 0, width <= 100, height <= 100, rgba.count == width * height * 4 else { return nil }
        let count = width * height
        var average = [Double](repeating: 0, count: 4)
        for i in 0..<count {
            let alpha = Double(rgba[i * 4 + 3]) / 255
            for c in 0..<3 { average[c] += alpha / 255 * Double(rgba[i * 4 + c]) }
            average[3] += alpha
        }
        if average[3] > 0 { for c in 0..<3 { average[c] /= average[3] } }
        let hasAlpha = average[3] < Double(count)
        let limit = hasAlpha ? 5.0 : 7.0
        let lx = max(1, Int((limit * Double(width) / Double(max(width, height))).rounded()))
        let ly = max(1, Int((limit * Double(height) / Double(max(width, height))).rounded()))
        var channels = [[Double]](repeating: [Double](repeating: 0, count: count), count: 4)
        for i in 0..<count {
            let alpha = Double(rgba[i * 4 + 3]) / 255
            let r = average[0] * (1 - alpha) + alpha / 255 * Double(rgba[i * 4])
            let g = average[1] * (1 - alpha) + alpha / 255 * Double(rgba[i * 4 + 1])
            let b = average[2] * (1 - alpha) + alpha / 255 * Double(rgba[i * 4 + 2])
            channels[0][i] = (r + g + b) / 3
            channels[1][i] = (r + g) / 2 - b
            channels[2][i] = r - g
            channels[3][i] = alpha
        }
        func channel(_ values: [Double], _ nx: Int, _ ny: Int) -> (Double, [Double], Double) {
            var dc = 0.0
            var ac: [Double] = []
            var scale = 0.0
            for cy in 0..<ny {
                var cx = 0
                while cx * ny < nx * (ny - cy) {
                    let fx = (0..<width).map { cos(.pi / Double(width) * Double(cx) * (Double($0) + 0.5)) }
                    var value = 0.0
                    for y in 0..<height {
                        let fy = cos(.pi / Double(height) * Double(cy) * (Double(y) + 0.5))
                        for x in 0..<width { value += values[x + y * width] * fx[x] * fy }
                    }
                    value /= Double(count)
                    if cx != 0 || cy != 0 { ac.append(value); scale = max(scale, abs(value)) } else { dc = value }
                    cx += 1
                }
            }
            if scale > 0 { ac = ac.map { 0.5 + 0.5 / scale * $0 } }
            return (dc, ac, scale)
        }
        let l = channel(channels[0], max(3, lx), max(3, ly))
        let p = channel(channels[1], 3, 3)
        let q = channel(channels[2], 3, 3)
        let a = hasAlpha ? channel(channels[3], 5, 5) : (0.0, [], 0.0)
        func round(_ value: Double) -> Int { Int(floor(value + 0.5)) }
        let header24 = round(63 * l.0) | (round(31.5 + 31.5 * p.0) << 6) | (round(31.5 + 31.5 * q.0) << 12) | (round(31 * l.2) << 18) | (hasAlpha ? 1 << 23 : 0)
        let header16 = (width > height ? ly : lx) | (round(63 * p.2) << 3) | (round(63 * q.2) << 9) | (width > height ? 1 << 15 : 0)
        var hash: [UInt8] = [UInt8(truncatingIfNeeded: header24), UInt8(truncatingIfNeeded: header24 >> 8), UInt8(truncatingIfNeeded: header24 >> 16), UInt8(truncatingIfNeeded: header16), UInt8(truncatingIfNeeded: header16 >> 8)]
        if hasAlpha { hash.append(UInt8(round(15 * a.0) | (round(15 * a.2) << 4))) }
        let start = hash.count
        var index = 0
        for values in hasAlpha ? [l.1, p.1, q.1, a.1] : [l.1, p.1, q.1] {
            for value in values {
                let position = start + index / 2
                if position == hash.count { hash.append(0) }
                hash[position] |= UInt8(round(15 * value) << ((index & 1) * 4))
                index += 1
            }
        }
        return Data(hash)
    }
}
