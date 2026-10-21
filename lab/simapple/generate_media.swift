import Foundation
import AVFoundation
import CoreGraphics
import CoreMedia
import CoreVideo
import ImageIO
import UniformTypeIdentifiers

let directory = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
let temporary = FileManager.default.temporaryDirectory.appendingPathComponent("apple-bridge-media-"+UUID().uuidString)
try FileManager.default.createDirectory(at: temporary, withIntermediateDirectories: true)
defer { try? FileManager.default.removeItem(at: temporary) }
func check(_ status: OSStatus) { precondition(status == noErr, "media status \(status)") }
func wait(_ input: AVAssetWriterInput) {
 let until = Date().addingTimeInterval(30)
 while !input.isReadyForMoreMediaData { precondition(Date() < until, "encoder stalled"); Thread.sleep(forTimeInterval: 0.001) }
}
func normalized(_ input: Data) -> Data {
 var data = input
 func number(_ offset: Int, _ count: Int) -> UInt64 { data[offset..<offset+count].reduce(0) { $0 << 8 | UInt64($1) } }
 func walk(_ start: Int, _ end: Int) {
  var position = start
  while position+8 <= end {
   let small = number(position,4), header = small == 1 ? 16 : 8
   let size = small == 0 ? end-position : Int(small == 1 ? number(position+8,8) : small)
   precondition(size >= header && position+size <= end, "invalid encoded box")
   let kind = String(data: data[position+4..<position+8], encoding: .ascii)!, body = position+header
   if ["mvhd","tkhd","mdhd"].contains(kind) { let count = data[body] == 1 ? 16 : 8; data.replaceSubrange(body+4..<body+4+count, with: repeatElement(UInt8(0), count: count)) }
   if ["moov","trak","mdia"].contains(kind) { walk(body,position+size) }
   position += size
  }
 }
 walk(0,data.count)
 return data
}
func encode(_ name: String, _ width: Int, _ height: Int, image: CGImage? = nil, audio: URL) throws -> URL {
 let file = temporary.appendingPathComponent(name+"-raw.mp4"), writer = try AVAssetWriter(outputURL: file, fileType: .mp4)
 writer.shouldOptimizeForNetworkUse = true; writer.metadata = []
 let video = AVAssetWriterInput(mediaType: .video, outputSettings: [AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: width, AVVideoHeightKey: height, AVVideoCompressionPropertiesKey: [AVVideoAverageBitRateKey: 100000, AVVideoExpectedSourceFrameRateKey: 12, AVVideoMaxKeyFrameIntervalKey: 12]])
 let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: video, sourcePixelBufferAttributes: [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA, kCVPixelBufferWidthKey as String: width, kCVPixelBufferHeightKey as String: height])
 precondition(writer.canAdd(video)); writer.add(video)
 precondition(writer.startWriting()); writer.startSession(atSourceTime: .zero)
 for frame in 0..<12 {
  wait(video); var pixel: CVPixelBuffer?
  check(CVPixelBufferCreate(kCFAllocatorDefault, width, height, kCVPixelFormatType_32BGRA, nil, &pixel))
  CVPixelBufferLockBaseAddress(pixel!, []); let address = CVPixelBufferGetBaseAddress(pixel!)!, stride = CVPixelBufferGetBytesPerRow(pixel!)
  if let image = image {
   let context = CGContext(data: address, width: width, height: height, bitsPerComponent: 8, bytesPerRow: stride, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGBitmapInfo.byteOrder32Little.rawValue | CGImageAlphaInfo.premultipliedFirst.rawValue)!
   context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
  } else {
   let bytes = address.assumingMemoryBound(to: UInt8.self), side = min(width,height)*4/5
   for y in 0..<height { for x in 0..<width { let red = abs(2*x-width+1) < side && abs(2*y-height+1) < side, offset = y*stride+x*4; bytes[offset] = red ? 0 : 200; bytes[offset+1] = red ? 0 : 76; bytes[offset+2] = red ? 255 : 0; bytes[offset+3] = 255 } }
  }
  CVPixelBufferUnlockBaseAddress(pixel!, []); precondition(adaptor.append(pixel!, withPresentationTime: CMTime(value: Int64(frame), timescale: 12)))
 }
 video.markAsFinished(); writer.endSession(atSourceTime: CMTime(value: 1, timescale: 1))
 let finished = DispatchSemaphore(value: 0); writer.finishWriting { finished.signal() }; precondition(finished.wait(timeout: .now()+30) == .success)
 precondition(writer.status == .completed, "\(String(describing: writer.error))")
 let composition = AVMutableComposition(), sources = [AVURLAsset(url: file), AVURLAsset(url: audio)]
 defer {withExtendedLifetime(sources) {}}
 for (asset,type) in zip(sources,[AVMediaType.video,AVMediaType.audio]) {
  let track = asset.tracks(withMediaType: type).first!
  fputs("\(name) \(type.rawValue) start=\(CMTimeGetSeconds(track.timeRange.start)) duration=\(CMTimeGetSeconds(track.timeRange.duration))\n",stderr)
  try composition.addMutableTrack(withMediaType: type, preferredTrackID: kCMPersistentTrackID_Invalid)!.insertTimeRange(CMTimeRange(start: .zero, duration: CMTime(value: 1, timescale: 1)), of: track, at: .zero)
 }
 let final = temporary.appendingPathComponent(name+".mp4"), export = AVAssetExportSession(asset: composition, presetName: AVAssetExportPresetPassthrough)!
 export.outputURL = final; export.outputFileType = .mp4; export.shouldOptimizeForNetworkUse = true; export.metadata = []
 let muxed = DispatchSemaphore(value: 0); export.exportAsynchronously {muxed.signal()}; precondition(muxed.wait(timeout: .now()+30) == .success)
 precondition(export.status == .completed, "\(String(describing: export.error))")
 let bytes = normalized(try Data(contentsOf: final)); precondition(bytes.count < 50000, "clip exceeds 50 KB"); try bytes.write(to: final); print("\(name) \(width)x\(height) \(bytes.count) bytes")
 return final
}
func fixture(_ kind: String = "") throws -> Data {
 let source = Process(), pipe = Pipe(); source.executableURL = URL(fileURLWithPath: "/usr/bin/env"); source.arguments = ["go","run","./lab/simapple/generate-media"]+(kind.isEmpty ? [] : [kind]); source.currentDirectoryURL = directory.deletingLastPathComponent().deletingLastPathComponent(); source.standardOutput = pipe
 try source.run(); let data = pipe.fileHandleForReading.readDataToEndOfFile(); source.waitUntilExit(); precondition(source.terminationStatus == 0); return data
}
let still = try fixture(), audio = temporary.appendingPathComponent("silence.m4a")
try fixture("audio").write(to: audio)
let image = CGImageSourceCreateImageAtIndex(CGImageSourceCreateWithData(still as CFData,nil)!,0,nil)!
var output = "package simapple\n\n"
for (name,width,height) in [("Landscape",320,180),("Portrait",180,320),("Panorama",1920,200),("Live",360,360)] {
 let file = try encode(name,width,height,image: name == "Live" ? image : nil, audio: audio), data = try Data(contentsOf: file)
 output += "const simVideo\(name) = \"\(data.base64EncodedString())\"\n\n"
 if name == "Live" {
  let frame = try AVAssetImageGenerator(asset: AVURLAsset(url: file)).copyCGImage(at: .zero, actualTime: nil), png = NSMutableData()
  let destination = CGImageDestinationCreateWithData(png,UTType.png.identifier as CFString,1,nil)!; CGImageDestinationAddImage(destination,frame,nil); precondition(CGImageDestinationFinalize(destination))
  output += "const simLiveStill = \"\((png as Data).base64EncodedString())\"\n\n"
 }
}
try output.write(to: directory.appendingPathComponent("media_video_clips.go"), atomically: true, encoding: .utf8)
