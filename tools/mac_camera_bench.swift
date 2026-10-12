import AVFoundation
import AppKit
import CoreMedia
import CoreVideo
import Darwin
import Foundation

private let nanosecondsPerSecond: UInt64 = 1_000_000_000

private enum BenchError: Error, CustomStringConvertible {
    case usage(String)
    case io(String)
    case camera(String)
    case ffi(String)
    case fixture(String)

    var description: String {
        switch self {
        case let .usage(message), let .io(message), let .camera(message), let .ffi(message),
             let .fixture(message):
            return message
        }
    }
}

private struct Options {
    var durationSeconds = 20.0
    var maxFrames = 600
    var warmupFrames = 30
    var prepareSeconds = 3.0
    var width = 640
    var height = 480
    var maxPixelReads = 64_000_000
    var cameraIndex = 0
    var outputPath: String?
    var expectedPayload: Data?
    var fixturePath: String?
    var help = false

    init(arguments: [String]) throws {
        var index = 1
        while index < arguments.count {
            let argument = arguments[index]
            switch argument {
            case "--help", "-h":
                help = true
            case "--duration-seconds":
                durationSeconds = try Self.doubleValue(arguments, &index, argument)
            case "--max-frames":
                maxFrames = try Self.intValue(arguments, &index, argument)
            case "--warmup-frames":
                warmupFrames = try Self.intValue(arguments, &index, argument)
            case "--prepare-seconds":
                prepareSeconds = try Self.doubleValue(arguments, &index, argument)
            case "--width":
                width = try Self.intValue(arguments, &index, argument)
            case "--height":
                height = try Self.intValue(arguments, &index, argument)
            case "--max-pixel-reads":
                maxPixelReads = try Self.intValue(arguments, &index, argument)
            case "--camera-index":
                cameraIndex = try Self.intValue(arguments, &index, argument)
            case "--output":
                outputPath = try Self.stringValue(arguments, &index, argument)
            case "--expected-payload":
                let value = try Self.stringValue(arguments, &index, argument)
                guard let data = value.data(using: .utf8) else {
                    throw BenchError.usage("--expected-payload must be valid UTF-8")
                }
                expectedPayload = data
            case "--fixture":
                fixturePath = try Self.stringValue(arguments, &index, argument)
            default:
                throw BenchError.usage("unknown option: \(argument)\n\n\(Self.usage)")
            }
            index += 1
        }
        guard durationSeconds > 0, durationSeconds.isFinite else {
            throw BenchError.usage("--duration-seconds must be finite and greater than zero")
        }
        guard maxFrames > 0 else {
            throw BenchError.usage("--max-frames must be greater than zero")
        }
        guard warmupFrames >= 0 else {
            throw BenchError.usage("--warmup-frames cannot be negative")
        }
        guard prepareSeconds >= 0, prepareSeconds.isFinite else {
            throw BenchError.usage("--prepare-seconds must be finite and non-negative")
        }
        guard (1...640).contains(width), (1...640).contains(height) else {
            throw BenchError.usage("--width and --height must be in 1...640 for the default FFI build")
        }
        guard maxPixelReads > 0 else {
            throw BenchError.usage("--max-pixel-reads must be greater than zero")
        }
        guard cameraIndex >= 0 else {
            throw BenchError.usage("--camera-index cannot be negative")
        }
    }

    static let usage = """
    Tessipher macOS camera benchmark

    The default command opens a live preview and waits for the Start measurement
    button. It records decoder timings and capture accounting only; camera frames
    and decoded payloads are never written. A JSON report is printed to stdout.

    Options:
      --duration-seconds N    measured duration (default: 20)
      --max-frames N         measured frame cap (default: 600)
      --warmup-frames N      frames decoded before timing (default: 30)
      --prepare-seconds N    preview delay after Start (default: 3)
      --width N              requested camera width (default: 640)
      --height N             requested camera height (default: 480)
      --max-pixel-reads N    per-call decoder budget (default: 64000000)
      --camera-index N       discovered camera index (default: 0)
      --expected-payload S   optional UTF-8 payload comparison
      --output PATH          also write the metrics JSON to PATH
      --fixture PATH         run a headless PGM fixture instead of opening a camera
      --help                 show this help

    The fixture path is intended for offline harness tests. Its rows are padded
    before decoding, which exercises the borrowed stride contract.
    """

    private static func value(_ arguments: [String], _ index: inout Int, _ option: String) throws -> String {
        let next = index + 1
        guard next < arguments.count else {
            throw BenchError.usage("missing value for \(option)")
        }
        index = next
        return arguments[next]
    }

    private static func intValue(_ arguments: [String], _ index: inout Int, _ option: String) throws -> Int {
        let value = try Self.value(arguments, &index, option)
        guard let parsed = Int(value) else {
            throw BenchError.usage("invalid integer for \(option): \(value)")
        }
        return parsed
    }

    private static func doubleValue(
        _ arguments: [String], _ index: inout Int, _ option: String
    ) throws -> Double {
        let value = try Self.value(arguments, &index, option)
        guard let parsed = Double(value) else {
            throw BenchError.usage("invalid number for \(option): \(value)")
        }
        return parsed
    }

    private static func stringValue(
        _ arguments: [String], _ index: inout Int, _ option: String
    ) throws -> String {
        try Self.value(arguments, &index, option)
    }
}

private struct DecodeObservation {
    let status: Int32
    let latencyMicroseconds: Double
    let pixelReads: UInt64
    let candidates: UInt64
    let retries: UInt64
    let passes: UInt64
    let expectedMatch: Bool?
}

private final class DecoderMetrics {
    let name: String
    let strategy: String
    let tracked: Bool
    private(set) var latencies: [Double] = []
    private(set) var pixelReads: [Double] = []
    private(set) var candidates: [Double] = []
    private(set) var retries: [Double] = []
    private(set) var passes: [Double] = []
    private(set) var statuses: [String: Int] = [:]
    private(set) var successes = 0
    private(set) var expectedMatches = 0
    private(set) var expectedMismatches = 0

    init(name: String, strategy: String, tracked: Bool) {
        self.name = name
        self.strategy = strategy
        self.tracked = tracked
    }

    func reset() {
        latencies.removeAll(keepingCapacity: true)
        pixelReads.removeAll(keepingCapacity: true)
        candidates.removeAll(keepingCapacity: true)
        retries.removeAll(keepingCapacity: true)
        passes.removeAll(keepingCapacity: true)
        statuses.removeAll(keepingCapacity: true)
        successes = 0
        expectedMatches = 0
        expectedMismatches = 0
    }

    func record(_ observation: DecodeObservation) {
        latencies.append(observation.latencyMicroseconds)
        pixelReads.append(Double(observation.pixelReads))
        candidates.append(Double(observation.candidates))
        retries.append(Double(observation.retries))
        passes.append(Double(observation.passes))
        let key = statusName(observation.status)
        statuses[key, default: 0] += 1
        if observation.status == 0 {
            successes += 1
        }
        if let expectedMatch = observation.expectedMatch {
            if expectedMatch {
                expectedMatches += 1
            } else {
                expectedMismatches += 1
            }
        }
    }

    func json(sampleCount: Int) -> [String: Any] {
        [
            "name": name,
            "strategy": strategy,
            "tracked": tracked,
            "sample_count": latencies.count,
            "success_count": successes,
            "decoder_success_fraction": sampleCount == 0 ? 0.0 : Double(successes) / Double(sampleCount),
            "status_counts": statuses,
            "latency_us": numericSummary(latencies),
            "stats": [
                "pixel_reads": numericSummary(pixelReads),
                "candidates": numericSummary(candidates),
                "retries": numericSummary(retries),
                "passes": numericSummary(passes),
            ],
            "expected_payload_matches": expectedMatches,
            "expected_payload_mismatches": expectedMismatches,
        ]
    }
}

private final class DecoderContext {
    let metrics: DecoderMetrics
    private let strategy: UInt32
    private let tracked: Bool
    private let detectorScratch: UnsafeMutablePointer<UInt8>
    private let coreScratch: UnsafeMutablePointer<UInt8>
    private let payload: UnsafeMutablePointer<UInt8>
    private let segments: UnsafeMutablePointer<qr_segment>
    private var tracker = qr_tracker()
    private var config: qr_config
    private let detectorScratchLength: Int
    private let coreScratchLength: Int
    private let payloadLength: Int
    private let segmentCapacity: Int

    init(memory: qr_memory_info, config: qr_config, strategy: UInt32, tracked: Bool) throws {
        self.strategy = strategy
        self.tracked = tracked
        self.config = config
        detectorScratchLength = memory.detector_scratch_bytes
        coreScratchLength = memory.core_scratch_bytes
        payloadLength = memory.max_payload_bytes
        segmentCapacity = memory.max_segments
        detectorScratch = UnsafeMutablePointer<UInt8>.allocate(
            capacity: max(1, Int(memory.detector_scratch_bytes)))
        coreScratch = UnsafeMutablePointer<UInt8>.allocate(capacity: max(1, Int(memory.core_scratch_bytes)))
        payload = UnsafeMutablePointer<UInt8>.allocate(capacity: max(1, Int(memory.max_payload_bytes)))
        segments = UnsafeMutablePointer<qr_segment>.allocate(capacity: max(1, Int(memory.max_segments)))
        let modeName = strategy == 0 ? "center" : "center_then_majority5"
        let prefix = tracked ? "tracker" : "stateless"
        metrics = DecoderMetrics(name: "\(prefix)_\(modeName)", strategy: modeName, tracked: tracked)
        guard qr_tracker_init(&tracker) == 0 else {
            detectorScratch.deallocate()
            coreScratch.deallocate()
            payload.deallocate()
            segments.deallocate()
            throw BenchError.ffi("qr_tracker_init rejected the benchmark tracker")
        }
    }

    deinit {
        detectorScratch.deallocate()
        coreScratch.deallocate()
        payload.deallocate()
        segments.deallocate()
    }

    func reset() {
        metrics.reset()
        _ = qr_tracker_reset(&tracker)
    }

    func decode(
        pixels: UnsafePointer<UInt8>,
        pixelsLength: Int,
        width: Int,
        height: Int,
        stride: Int,
        maxPixelReads: Int,
        expectedPayload: Data?
    ) -> DecodeObservation {
        var result = qr_result()
        var stats = qr_stats()
        let start = DispatchTime.now().uptimeNanoseconds
        let status: Int32
        if tracked {
            status = withUnsafeMutablePointer(to: &tracker) { trackerPointer in
                withUnsafePointer(to: &config) { configPointer in
                    withUnsafeMutablePointer(to: &result) { resultPointer in
                        withUnsafeMutablePointer(to: &stats) { statsPointer in
                            qr_tracker_decode_gray(
                                trackerPointer,
                                configPointer,
                                maxPixelReads,
                                strategy,
                                pixels,
                                pixelsLength,
                                width,
                                height,
                                stride,
                                detectorScratch,
                                detectorScratchLength,
                                coreScratch,
                                coreScratchLength,
                                payload,
                                payloadLength,
                                segments,
                                segmentCapacity,
                                resultPointer,
                                statsPointer)
                        }
                    }
                }
            }
        } else {
            status = withUnsafePointer(to: &config) { configPointer in
                withUnsafeMutablePointer(to: &result) { resultPointer in
                    withUnsafeMutablePointer(to: &stats) { statsPointer in
                        qr_decode_gray_with_sampling(
                            configPointer,
                            maxPixelReads,
                            strategy,
                            pixels,
                            pixelsLength,
                            width,
                            height,
                            stride,
                            detectorScratch,
                            detectorScratchLength,
                            coreScratch,
                            coreScratchLength,
                            payload,
                            payloadLength,
                            segments,
                            segmentCapacity,
                            resultPointer,
                            statsPointer)
                    }
                }
            }
        }
        let elapsed = DispatchTime.now().uptimeNanoseconds - start
        var expectedMatch: Bool?
        if let expectedPayload, status == 0 {
            let payloadCount = Int(result.payload_len)
            expectedMatch = payloadCount == expectedPayload.count
                && expectedPayload.withUnsafeBytes { expectedBytes in
                    guard let baseAddress = expectedBytes.baseAddress else { return payloadCount == 0 }
                    return memcmp(payload, baseAddress, payloadCount) == 0
                }
        }
        return DecodeObservation(
            status: status,
            latencyMicroseconds: Double(elapsed) / 1_000.0,
            pixelReads: UInt64(stats.pixel_reads),
            candidates: UInt64(stats.candidates),
            retries: UInt64(stats.retries),
            passes: UInt64(stats.passes),
            expectedMatch: expectedMatch)
    }
}

private struct PreparationPlan {
    let width: Int
    let height: Int
    let copied: Bool
    let method: String
}

/// Converts a camera luma plane to an aspect-preserving Gray8 image no larger
/// than the FFI build's configured limit. The destination buffer is reused for
/// every frame and remains borrowed only through the decode closure.
private final class Gray8Preparer {
    private let targetWidth: Int
    private let targetHeight: Int
    private var buffer: [UInt8] = []

    init(targetWidth: Int, targetHeight: Int) {
        self.targetWidth = targetWidth
        self.targetHeight = targetHeight
    }

    func plan(sourceWidth: Int, sourceHeight: Int) -> PreparationPlan {
        guard sourceWidth > 0, sourceHeight > 0 else {
            return PreparationPlan(width: 0, height: 0, copied: false, method: "invalid")
        }
        guard sourceWidth > targetWidth || sourceHeight > targetHeight else {
            return PreparationPlan(
                width: sourceWidth,
                height: sourceHeight,
                copied: false,
                method: "borrowed_gray8")
        }
        let scale = min(
            Double(targetWidth) / Double(sourceWidth),
            Double(targetHeight) / Double(sourceHeight))
        let width = max(1, Int((Double(sourceWidth) * scale).rounded(.down)))
        let height = max(1, Int((Double(sourceHeight) * scale).rounded(.down)))
        return PreparationPlan(width: width, height: height, copied: true, method: "nearest_aspect_fit")
    }

    func withPreparedFrame(
        source: UnsafePointer<UInt8>,
        sourceWidth: Int,
        sourceHeight: Int,
        sourceStride: Int,
        body: (UnsafePointer<UInt8>, Int, Int, Int, Int, PreparationPlan) -> Void
    ) {
        let plan = plan(sourceWidth: sourceWidth, sourceHeight: sourceHeight)
        guard plan.copied else {
            let sourceLength = (sourceHeight - 1) * sourceStride + sourceWidth
            body(source, sourceLength, plan.width, plan.height, sourceStride, plan)
            return
        }
        let destinationStride = plan.width
        let required = destinationStride * plan.height
        if buffer.count != required {
            buffer = [UInt8](repeating: 255, count: required)
        }
        for destinationY in 0..<plan.height {
            let sourceY = min(sourceHeight - 1, destinationY * sourceHeight / plan.height)
            let sourceRow = sourceY * sourceStride
            let destinationRow = destinationY * destinationStride
            for destinationX in 0..<plan.width {
                let sourceX = min(sourceWidth - 1, destinationX * sourceWidth / plan.width)
                buffer[destinationRow + destinationX] = source[sourceRow + sourceX]
            }
        }
        buffer.withUnsafeBufferPointer { destination in
            body(
                destination.baseAddress!,
                required,
                plan.width,
                plan.height,
                destinationStride,
                plan)
        }
    }
}

private final class MeasurementRunner {
    let options: Options
    let memory: qr_memory_info
    private let contexts: [DecoderContext]
    private let preparer: Gray8Preparer
    private(set) var active = false
    private(set) var finalJSON: Data?
    private var completionReason = "not_started"
    private var runStartedNanoseconds: UInt64 = 0
    private var measuredStartedNanoseconds: UInt64?
    private var lastMeasuredArrivalNanoseconds: UInt64?
    private var warmupSeen = 0
    private(set) var sampleCount = 0
    private(set) var measuredDroppedFrames = 0
    private(set) var warmupDroppedFrames = 0
    private(set) var measuredUnsupportedFrames = 0
    private(set) var warmupUnsupportedFrames = 0
    private var interarrivalMicroseconds: [Double] = []
    private var sourceWidth = 0
    private var sourceHeight = 0
    private var sourceStride = 0
    private var sourcePixelFormat = "unknown"
    private var preparedWidth = 0
    private var preparedHeight = 0
    private var preparedStride = 0
    private var preparationMethod = "unknown"
    private var preparationCopied = false
    private var preparationMicroseconds: [Double] = []
    private var cameraName: String?
    private var cameraPosition: String?
    private(set) var onFinished: ((Data) -> Void)?

    init(options: Options) throws {
        self.options = options
        var configuration = qr_config()
        guard qr_default_config(&configuration) == 0 else {
            throw BenchError.ffi("qr_default_config failed")
        }
        var queriedMemory = qr_memory_info()
        let memoryStatus = withUnsafePointer(to: &configuration) { configPointer in
            withUnsafeMutablePointer(to: &queriedMemory) { memoryPointer in
                qr_memory(configPointer, memoryPointer)
            }
        }
        guard memoryStatus == 0 else {
            throw BenchError.ffi("qr_memory failed with status \(memoryStatus)")
        }
        memory = queriedMemory
        let contextDefinitions: [(UInt32, Bool)] = [
            (0, false),
            (1, false),
            (0, true),
            (1, true),
        ]
        contexts = try contextDefinitions.map { strategy, tracked in
            try DecoderContext(memory: queriedMemory, config: configuration, strategy: strategy, tracked: tracked)
        }
        preparer = Gray8Preparer(targetWidth: options.width, targetHeight: options.height)
    }

    func setCompletion(_ completion: @escaping (Data) -> Void) {
        onFinished = completion
    }

    func setCameraMetadata(name: String, position: String) {
        cameraName = name
        cameraPosition = position
    }

    func preparationPlan(sourceWidth: Int, sourceHeight: Int) -> PreparationPlan {
        preparer.plan(sourceWidth: sourceWidth, sourceHeight: sourceHeight)
    }

    func begin() {
        for context in contexts {
            context.reset()
        }
        active = true
        finalJSON = nil
        completionReason = "running"
        runStartedNanoseconds = DispatchTime.now().uptimeNanoseconds
        measuredStartedNanoseconds = nil
        lastMeasuredArrivalNanoseconds = nil
        warmupSeen = 0
        sampleCount = 0
        measuredDroppedFrames = 0
        warmupDroppedFrames = 0
        measuredUnsupportedFrames = 0
        warmupUnsupportedFrames = 0
        interarrivalMicroseconds.removeAll(keepingCapacity: true)
        sourceWidth = 0
        sourceHeight = 0
        sourceStride = 0
        sourcePixelFormat = "unknown"
        preparedWidth = 0
        preparedHeight = 0
        preparedStride = 0
        preparationMethod = "unknown"
        preparationCopied = false
        preparationMicroseconds.removeAll(keepingCapacity: true)
    }

    func recordDrop() {
        guard active else { return }
        if warmupSeen < options.warmupFrames {
            warmupDroppedFrames += 1
        } else {
            measuredDroppedFrames += 1
        }
    }

    func recordUnsupportedFrame() {
        guard active else { return }
        if warmupSeen < options.warmupFrames {
            warmupUnsupportedFrames += 1
        } else {
            measuredUnsupportedFrames += 1
        }
    }

    func processFrame(
        pixels: UnsafePointer<UInt8>,
        pixelsLength: Int,
        width: Int,
        height: Int,
        stride: Int,
        pixelFormat: String
    ) {
        guard active else { return }
        let arrival = DispatchTime.now().uptimeNanoseconds
        guard width > 0, height > 0, stride >= width else { return }
        let requiredSourceLength = (height - 1) * stride + width
        guard pixelsLength >= requiredSourceLength else { return }
        sourceWidth = width
        sourceHeight = height
        sourceStride = stride
        sourcePixelFormat = pixelFormat
        let preparationStart = DispatchTime.now().uptimeNanoseconds
        preparer.withPreparedFrame(
            source: pixels,
            sourceWidth: width,
            sourceHeight: height,
            sourceStride: stride) { [self] preparedPixels, preparedLength, preparedWidth, preparedHeight, preparedStride, plan in
                let preparationElapsed = DispatchTime.now().uptimeNanoseconds - preparationStart
                self.preparedWidth = preparedWidth
                self.preparedHeight = preparedHeight
                self.preparedStride = preparedStride
                self.preparationMethod = plan.method
                self.preparationCopied = plan.copied
                if self.warmupSeen < self.options.warmupFrames {
                    self.warmupSeen += 1
                    self.runContexts(
                        pixels: preparedPixels,
                        pixelsLength: preparedLength,
                        width: preparedWidth,
                        height: preparedHeight,
                        stride: preparedStride,
                        measuredIndex: self.warmupSeen - 1,
                        expectedPayload: self.options.expectedPayload,
                        record: false)
                    if self.warmupSeen == self.options.warmupFrames {
                        self.lastMeasuredArrivalNanoseconds = nil
                    }
                    return
                }
                if self.measuredStartedNanoseconds == nil {
                    self.measuredStartedNanoseconds = arrival
                }
                if let previous = self.lastMeasuredArrivalNanoseconds {
                    self.interarrivalMicroseconds.append(Double(arrival - previous) / 1_000.0)
                }
                self.lastMeasuredArrivalNanoseconds = arrival
                self.preparationMicroseconds.append(Double(preparationElapsed) / 1_000.0)
                self.runContexts(
                    pixels: preparedPixels,
                    pixelsLength: preparedLength,
                    width: preparedWidth,
                    height: preparedHeight,
                    stride: preparedStride,
                    measuredIndex: self.sampleCount,
                    expectedPayload: self.options.expectedPayload,
                    record: true)
                self.sampleCount += 1
                if self.sampleCount >= self.options.maxFrames {
                    self.finish(reason: "frame_cap")
                } else {
                    self.checkTimeout(now: arrival)
                }
            }
    }

    func stop(reason: String) {
        guard active else { return }
        finish(reason: reason)
    }

    private func checkTimeout(now: UInt64) {
        guard let measuredStartedNanoseconds else { return }
        let elapsed = Double(now - measuredStartedNanoseconds) / Double(nanosecondsPerSecond)
        if elapsed >= options.durationSeconds {
            finish(reason: "duration")
        }
    }

    private func runContexts(
        pixels: UnsafePointer<UInt8>,
        pixelsLength: Int,
        width: Int,
        height: Int,
        stride: Int,
        measuredIndex: Int,
        expectedPayload: Data?,
        record: Bool
    ) {
        let order: [Int] = measuredIndex.isMultiple(of: 2) ? [0, 1] : [1, 0]
        for group in 0..<2 {
            for strategy in order {
                let context = contexts[group * 2 + strategy]
                let observation = context.decode(
                    pixels: pixels,
                    pixelsLength: pixelsLength,
                    width: width,
                    height: height,
                    stride: stride,
                    maxPixelReads: options.maxPixelReads,
                    expectedPayload: expectedPayload)
                if record {
                    context.metrics.record(observation)
                }
            }
        }
    }

    private func finish(reason: String) {
        guard active else { return }
        active = false
        completionReason = reason
        let report = makeReport()
        finalJSON = report
        onFinished?(report)
    }

    private func makeReport() -> Data {
        let now = DispatchTime.now().uptimeNanoseconds
        let elapsed = Double(now - runStartedNanoseconds) / Double(nanosecondsPerSecond)
        let measuredElapsed: Double
        if let measuredStartedNanoseconds {
            measuredElapsed = Double(now - measuredStartedNanoseconds) / Double(nanosecondsPerSecond)
        } else {
            measuredElapsed = 0
        }
        var report: [String: Any] = [
            "schema_version": 1,
            "benchmark": "tessipher_mac_camera",
            "generated_at_utc": ISO8601DateFormatter().string(from: Date()),
            "completion_reason": completionReason,
            "duration_requested_seconds": options.durationSeconds,
            "elapsed_seconds": elapsed,
            "measured_elapsed_seconds": measuredElapsed,
            "warmup_frames_requested": options.warmupFrames,
            "warmup_frames_decoded": warmupSeen,
            "sample_count": sampleCount,
            "max_frames": options.maxFrames,
            "timing_window_note": "Measured duration and decoder quantiles start at the first frame after warmup; the app keeps a separate 30-second minimum safety timeout for capture stalls.",
            "capture": [
                "delivered_frames": sampleCount,
                "warmup_delivered_frames": warmupSeen,
                "delivered_frames_total": sampleCount + warmupSeen,
                "dropped_frames": measuredDroppedFrames,
                "warmup_dropped_frames": warmupDroppedFrames,
                "unsupported_format_frames": measuredUnsupportedFrames,
                "warmup_unsupported_format_frames": warmupUnsupportedFrames,
                "interarrival_us": numericSummary(interarrivalMicroseconds),
            ],
            "input": [
                "requested_width": options.width,
                "requested_height": options.height,
                "source_width": sourceWidth,
                "source_height": sourceHeight,
                "source_stride_bytes": sourceStride,
                "source_pixel_format": sourcePixelFormat,
                "prepared_width": preparedWidth,
                "prepared_height": preparedHeight,
                "prepared_stride_bytes": preparedStride,
                "prepared_pixel_format": "gray8",
                "preparation_method": preparationMethod,
                "preparation_copied": preparationCopied,
                "preparation_us": numericSummary(preparationMicroseconds),
                "width": preparedWidth,
                "height": preparedHeight,
                "stride_bytes": preparedStride,
                "pixel_format": "gray8",
                "borrowed_without_copy": !preparationCopied,
                "source_borrowed_while_locked": options.fixturePath == nil,
            ],
            "decoder": contexts.map { $0.metrics.json(sampleCount: sampleCount) },
            "expected_payload_configured": options.expectedPayload != nil,
            "expected_payload_note": "A match checks the optional expected bytes; decoder_success_fraction is status success, not recognition accuracy.",
            "hardware": hardwareMetadata(),
            "compiler": [
                "swift": ProcessInfo.processInfo.environment["TESSIPHER_SWIFT_COMPILER"] ?? "unknown",
                "rust": ProcessInfo.processInfo.environment["TESSIPHER_RUSTC"] ?? "unknown",
            ],
        ]
        if let fixturePath = options.fixturePath {
            report["source"] = ["kind": "fixture", "path": fixturePath]
        } else {
            var camera: [String: Any] = ["kind": "camera", "camera_index": options.cameraIndex]
            if let cameraName { camera["name"] = cameraName }
            if let cameraPosition { camera["position"] = cameraPosition }
            report["source"] = camera
        }
        do {
            return try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys])
        } catch {
            return Data("{\"error\":\"could not serialize benchmark report\"}\n".utf8)
        }
    }
}

private func statusName(_ status: Int32) -> String {
    switch status {
    case 0: return "ok"
    case 1: return "invalid_argument"
    case 2: return "overlap"
    case 10: return "invalid_image"
    case 11: return "image_too_large"
    case 12: return "invalid_config"
    case 13: return "detector_scratch_too_small"
    case 14: return "finder_capacity"
    case 15: return "candidate_budget"
    case 16: return "retry_budget"
    case 17: return "not_found"
    case 18: return "pixel_budget"
    case 101: return "invalid_matrix"
    case 102: return "core_scratch_too_small"
    case 103: return "format"
    case 104: return "version"
    case 105: return "reed_solomon"
    case 106: return "truncated"
    case 107: return "unsupported_mode"
    case 108: return "malformed_payload"
    case 109: return "output_too_small"
    case 110: return "segments_too_small"
    default: return "status_\(status)"
    }
}

private func numericSummary(_ values: [Double]) -> [String: Any] {
    guard !values.isEmpty else { return ["count": 0] }
    let sorted = values.sorted()
    func quantile(_ q: Double) -> Double {
        let position = q * Double(sorted.count - 1)
        let low = Int(floor(position))
        let high = Int(ceil(position))
        if low == high { return sorted[low] }
        let fraction = position - Double(low)
        return sorted[low] + (sorted[high] - sorted[low]) * fraction
    }
    return [
        "count": sorted.count,
        "min": sorted[0],
        "p50": quantile(0.50),
        "p95": quantile(0.95),
        "p99": quantile(0.99),
        "max": sorted[sorted.count - 1],
    ]
}

private func hardwareMetadata() -> [String: Any] {
    func sysctlString(_ name: String) -> String? {
        var size = 0
        guard sysctlbyname(name, nil, &size, nil, 0) == 0, size > 0 else { return nil }
        var bytes = [UInt8](repeating: 0, count: size)
        guard sysctlbyname(name, &bytes, &size, nil, 0) == 0 else { return nil }
        return String(cString: bytes)
    }
    func sysctlUInt64(_ name: String) -> UInt64? {
        var value: UInt64 = 0
        var size = MemoryLayout<UInt64>.size
        guard sysctlbyname(name, &value, &size, nil, 0) == 0 else { return nil }
        return value
    }
    var result: [String: Any] = [
        "os": ProcessInfo.processInfo.operatingSystemVersionString,
        "model": sysctlString("hw.model") ?? "unknown",
    ]
    if let cpu = sysctlString("machdep.cpu.brand_string") { result["cpu"] = cpu }
    if let cores = sysctlUInt64("hw.ncpu") { result["logical_cores"] = cores }
    if let memory = sysctlUInt64("hw.memsize") { result["memory_bytes"] = memory }
    return result
}

private struct PGMImage {
    let width: Int
    let height: Int
    let stride: Int
    let pixels: [UInt8]
}

private func readPGM(_ path: String) throws -> PGMImage {
    let bytes = Array(try Data(contentsOf: URL(fileURLWithPath: path)))
    var index = 0
    func token() throws -> String {
        while index < bytes.count {
            if bytes[index] == 35 { // '#'
                while index < bytes.count, bytes[index] != 10 { index += 1 }
            } else if bytes[index] == 9 || bytes[index] == 10 || bytes[index] == 13 || bytes[index] == 32 {
                index += 1
            } else {
                break
            }
        }
        let start = index
        while index < bytes.count,
              bytes[index] != 9,
              bytes[index] != 10,
              bytes[index] != 13,
              bytes[index] != 32,
              bytes[index] != 35 {
            index += 1
        }
        guard start < index else { throw BenchError.fixture("malformed PGM token in \(path)") }
        return String(decoding: bytes[start..<index], as: UTF8.self)
    }
    guard try token() == "P5" else { throw BenchError.fixture("\(path) is not a binary P5 PGM") }
    guard let width = Int(try token()), let height = Int(try token()), let maxValue = Int(try token()),
          width > 0, height > 0, maxValue == 255 else {
        throw BenchError.fixture("\(path) has unsupported PGM dimensions or max value")
    }
    while index < bytes.count, bytes[index] == 9 || bytes[index] == 10 || bytes[index] == 13 || bytes[index] == 32 {
        index += 1
        break
    }
    let expected = width * height
    guard bytes.count - index >= expected else { throw BenchError.fixture("\(path) is truncated") }
    let stride = width + 16
    var pixels = [UInt8](repeating: 255, count: stride * height)
    for row in 0..<height {
        pixels.replaceSubrange(
            row * stride..<(row * stride + width),
            with: bytes[index + row * width..<(index + (row + 1) * width)])
    }
    return PGMImage(width: width, height: height, stride: stride, pixels: pixels)
}

private func writeReport(_ data: Data, outputPath: String?) throws {
    if let outputPath {
        try data.write(to: URL(fileURLWithPath: outputPath), options: .atomic)
    }
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data("\n".utf8))
}

private func runFixture(_ options: Options) throws {
    guard let fixturePath = options.fixturePath else { throw BenchError.fixture("fixture path is missing") }
    let image = try readPGM(fixturePath)
    let runner = try MeasurementRunner(options: options)
    runner.begin()
    image.pixels.withUnsafeBufferPointer { pixels in
        for _ in 0..<options.maxFrames where runner.active {
            runner.processFrame(
                pixels: pixels.baseAddress!,
                pixelsLength: image.pixels.count,
                width: image.width,
                height: image.height,
                stride: image.stride,
                pixelFormat: "PGM-gray8")
        }
    }
    runner.stop(reason: "fixture_complete")
    guard let report = runner.finalJSON else { throw BenchError.fixture("fixture produced no report") }
    try writeReport(report, outputPath: options.outputPath)
}

private final class PreviewView: NSView {
    var previewLayer: AVCaptureVideoPreviewLayer? {
        didSet { oldValue?.removeFromSuperlayer(); if let previewLayer { layer?.addSublayer(previewLayer) } }
    }

    override func layout() {
        super.layout()
        previewLayer?.frame = bounds
    }
}

private final class CameraAppDelegate: NSObject, NSApplicationDelegate, AVCaptureVideoDataOutputSampleBufferDelegate {
    private let options: Options
    private let session = AVCaptureSession()
    private let captureQueue = DispatchQueue(label: "com.nG7ypFxs.TessipherCameraBench.capture")
    private var output: AVCaptureVideoDataOutput?
    private var runner: MeasurementRunner?
    private var window: NSWindow?
    private var previewView: PreviewView?
    private var startButton: NSButton?
    private var stopButton: NSButton?
    private var statusField: NSTextField?
    // These fields are owned by captureQueue. UI actions enqueue state changes there.
    private var preparationWorkItem: DispatchWorkItem?
    private var preparationGeneration = 0
    private var timeoutWorkItem: DispatchWorkItem?
    private var inputFormatValidated = false
    private var inputFormatFailureReported = false

    init(options: Options) {
        self.options = options
        super.init()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        do {
            buildWindow()
            let createdRunner = try MeasurementRunner(options: options)
            createdRunner.setCompletion { [weak self] report in
                DispatchQueue.main.async { [weak self] in self?.measurementFinished(report) }
            }
            runner = createdRunner
            requestCameraAndStart()
        } catch {
            showFailure(error.localizedDescription)
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }

    func applicationWillTerminate(_ notification: Notification) {
        captureQueue.sync {
            preparationGeneration &+= 1
            preparationWorkItem?.cancel()
            preparationWorkItem = nil
            timeoutWorkItem?.cancel()
            timeoutWorkItem = nil
            runner?.stop(reason: "application_terminate")
            session.stopRunning()
        }
    }

    private func buildWindow() {
        let frame = NSRect(x: 0, y: 0, width: 960, height: 620)
        let window = NSWindow(
            contentRect: frame,
            styleMask: [.titled, .closable, .resizable, .miniaturizable],
            backing: .buffered,
            defer: false)
        window.title = "Tessipher Camera Benchmark"
        window.isReleasedWhenClosed = false
        let root = NSView(frame: frame)
        root.autoresizingMask = [.width, .height]
        let preview = PreviewView(frame: NSRect(x: 0, y: 70, width: frame.width, height: frame.height - 70))
        preview.autoresizingMask = [.width, .height]
        preview.wantsLayer = true
        root.addSubview(preview)
        let status = NSTextField(labelWithString: "Requesting camera access…")
        status.frame = NSRect(x: 20, y: 39, width: 700, height: 22)
        status.autoresizingMask = [.width, .minYMargin]
        root.addSubview(status)
        let start = NSButton(title: "Start measurement", target: self, action: #selector(startMeasurement))
        start.frame = NSRect(x: frame.width - 240, y: 35, width: 100, height: 30)
        start.autoresizingMask = [.minXMargin, .minYMargin]
        start.isEnabled = false
        root.addSubview(start)
        let stop = NSButton(title: "Stop", target: self, action: #selector(stopMeasurement))
        stop.frame = NSRect(x: frame.width - 130, y: 35, width: 100, height: 30)
        stop.autoresizingMask = [.minXMargin, .minYMargin]
        stop.isEnabled = false
        root.addSubview(stop)
        window.contentView = root
        window.center()
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        self.window = window
        previewView = preview
        statusField = status
        startButton = start
        stopButton = stop
    }

    private func requestCameraAndStart() {
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized:
            configureCapture()
        case .notDetermined:
            statusField?.stringValue = "Allow camera access when macOS asks…"
            AVCaptureDevice.requestAccess(for: .video) { [weak self] granted in
                DispatchQueue.main.async {
                    guard let self else { return }
                    if granted { self.configureCapture() }
                    else { self.showFailure("Camera access was denied. Enable it in System Settings → Privacy & Security → Camera, then rerun.") }
                }
            }
        case .denied, .restricted:
            showFailure("Camera access is unavailable. Enable this app in System Settings → Privacy & Security → Camera, then rerun.")
        @unknown default:
            showFailure("macOS returned an unknown camera authorization state.")
        }
    }

    private func configureCapture() {
        let devices = AVCaptureDevice.DiscoverySession(
            deviceTypes: [.builtInWideAngleCamera, .external],
            mediaType: .video,
            position: .unspecified).devices
        guard options.cameraIndex < devices.count else {
            showFailure("No camera at index \(options.cameraIndex). Found \(devices.count) camera(s).")
            return
        }
        let camera = devices[options.cameraIndex]
        let position: String
        switch camera.position {
        case .front: position = "front"
        case .back: position = "back"
        default: position = "unspecified"
        }
        runner?.setCameraMetadata(name: camera.localizedName, position: position)
        do {
            let input = try AVCaptureDeviceInput(device: camera)
            let output = AVCaptureVideoDataOutput()
            output.alwaysDiscardsLateVideoFrames = true
            output.videoSettings = [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange]
            output.setSampleBufferDelegate(self, queue: captureQueue)
            self.output = output
            let previewLayer = AVCaptureVideoPreviewLayer(session: session)
            previewLayer.videoGravity = .resizeAspect
            previewView?.previewLayer = previewLayer
            statusField?.stringValue = "Starting camera preview…"
            captureQueue.async { [weak self] in
                guard let self else { return }
                do {
                    self.inputFormatValidated = false
                    self.inputFormatFailureReported = false
                    self.session.beginConfiguration()
                    if self.session.canAddInput(input) {
                        self.session.addInput(input)
                    } else {
                        throw BenchError.camera("Cannot add camera input")
                    }
                    if self.session.canAddOutput(output) {
                        self.session.addOutput(output)
                    } else {
                        throw BenchError.camera("Cannot add camera output")
                    }
                    self.configureFormat(camera)
                    if let connection = output.connection(with: .video) { connection.isVideoMirrored = false }
                    self.session.commitConfiguration()
                    self.session.startRunning()
                } catch {
                    self.session.commitConfiguration()
                    DispatchQueue.main.async { [weak self] in self?.showFailure(error.localizedDescription) }
                }
            }
        } catch {
            showFailure(error.localizedDescription)
        }
    }

    private func configureFormat(_ camera: AVCaptureDevice) {
        let requestedWidth = Int32(options.width)
        let requestedHeight = Int32(options.height)
        // Prefer a small capture preset, but keep the pipeline usable when a
        // camera/driver returns a larger active format. The locked luma plane
        // is then reduced to the requested bound by Gray8Preparer while the
        // pixel buffer remains locked.
        if session.canSetSessionPreset(.vga640x480) {
            session.sessionPreset = .vga640x480
        }
        guard let format = camera.formats.first(where: {
            let dimensions = CMVideoFormatDescriptionGetDimensions($0.formatDescription)
            return dimensions.width == requestedWidth && dimensions.height == requestedHeight
        }) else { return }
        do {
            try camera.lockForConfiguration()
            defer { camera.unlockForConfiguration() }
            camera.activeFormat = format
            camera.activeVideoMinFrameDuration = CMTime(value: 1, timescale: 30)
            camera.activeVideoMaxFrameDuration = CMTime(value: 1, timescale: 30)
        } catch {
            // A format lock can fail while the session is starting. The
            // session preset and bounded software preparation remain valid.
        }
    }

    private func cameraReady(
        sourceWidth: Int,
        sourceHeight: Int,
        sourceStride: Int,
        plan: PreparationPlan
    ) {
        startButton?.isEnabled = true
        statusField?.stringValue =
            "Preview ready (source \(sourceWidth)×\(sourceHeight) → prepared \(plan.width)×\(plan.height), "
                + "luma stride \(sourceStride) → \(plan.width), \(plan.method)). "
                + "Place a QR code in view, then press Start measurement."
    }

    @objc private func startMeasurement() {
        guard let runner else { return }
        let canStart = captureQueue.sync {
            !runner.active && preparationWorkItem == nil
        }
        guard canStart else { return }
        startButton?.isEnabled = false
        stopButton?.isEnabled = true
        statusField?.stringValue = "Hold the QR code steady… starting in \(String(format: "%.1f", options.prepareSeconds)) s"
        let delay = DispatchTimeInterval.nanoseconds(Int(options.prepareSeconds * Double(nanosecondsPerSecond)))
        let preparation: DispatchWorkItem
        preparation = captureQueue.sync {
            preparationGeneration &+= 1
            let currentGeneration = preparationGeneration
            let item = DispatchWorkItem { [weak self, weak runner] in
                guard let self, let runner else { return }
                self.statusField?.stringValue = "Measuring; keep the QR code inside the preview."
                self.captureQueue.async { [weak self, weak runner] in
                    guard let self, let runner,
                          self.preparationGeneration == currentGeneration,
                          let pending = self.preparationWorkItem,
                          !pending.isCancelled else { return }
                    self.preparationWorkItem = nil
                    if !self.session.isRunning {
                        self.session.startRunning()
                    }
                    runner.begin()
                    let timeout = DispatchWorkItem { [weak self, weak runner] in
                        guard let self, let runner else { return }
                        self.captureQueue.async { runner.stop(reason: "timeout") }
                    }
                    self.timeoutWorkItem = timeout
                    let safetySeconds = max(30.0, runner.options.durationSeconds + 5.0)
                    DispatchQueue.main.asyncAfter(
                        deadline: .now() + safetySeconds,
                        execute: timeout)
                }
            }
            preparationWorkItem = item
            return item
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: preparation)
    }

    @objc private func stopMeasurement() {
        captureQueue.async { [weak self] in
            guard let self else { return }
            let wasActive = self.runner?.active ?? false
            self.preparationGeneration &+= 1
            self.preparationWorkItem?.cancel()
            self.preparationWorkItem = nil
            if wasActive {
                self.runner?.stop(reason: "user_stop")
            } else {
                DispatchQueue.main.async { [weak self] in
                    guard let self else { return }
                    self.startButton?.isEnabled = true
                    self.stopButton?.isEnabled = false
                    self.statusField?.stringValue = "Measurement canceled before start. Place the QR code and press Start measurement."
                }
            }
        }
    }

    private func showFailure(_ message: String) {
        statusField?.stringValue = message
        startButton?.isEnabled = false
        stopButton?.isEnabled = false
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = "Tessipher camera benchmark cannot start"
        alert.informativeText = message
        alert.addButton(withTitle: "OK")
        if window?.isVisible == true { alert.beginSheetModal(for: window!) }
    }

    private func measurementFinished(_ report: Data) {
        captureQueue.async { [weak self] in
            guard let self else { return }
            self.timeoutWorkItem?.cancel()
            self.timeoutWorkItem = nil
            self.session.stopRunning()
        }
        startButton?.isEnabled = true
        stopButton?.isEnabled = false
        statusField?.stringValue = "Measurement complete. JSON metrics were printed (and saved if --output was supplied)."
        do {
            try writeReport(report, outputPath: options.outputPath)
        } catch {
            showFailure("Measurement completed, but the JSON report could not be written: \(error)")
        }
    }

    // Called only on captureQueue so that session shutdown and the one-shot
    // failure guard cannot race a sample-buffer callback.
    private func rejectInputFormat(_ message: String) {
        guard !inputFormatFailureReported else { return }
        inputFormatFailureReported = true
        session.stopRunning()
        DispatchQueue.main.async { [weak self] in self?.showFailure(message) }
    }

    func captureOutput(_ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer, from connection: AVCaptureConnection) {
        guard let runner, let imageBuffer = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
        let lockStatus = CVPixelBufferLockBaseAddress(imageBuffer, .readOnly)
        guard lockStatus == kCVReturnSuccess else { return }
        defer { CVPixelBufferUnlockBaseAddress(imageBuffer, .readOnly) }
        if !runner.active && inputFormatValidated { return }
        let formatType = CVPixelBufferGetPixelFormatType(imageBuffer)
        let isVideoRange = formatType == kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
        let isFullRange = formatType == kCVPixelFormatType_420YpCbCr8BiPlanarFullRange
        guard CVPixelBufferIsPlanar(imageBuffer), isVideoRange || isFullRange else {
            if runner.active {
                runner.recordUnsupportedFrame()
            } else {
                rejectInputFormat("The camera did not deliver an 8-bit YUV luma plane in the requested format.")
            }
            return
        }
        let baseAddress = CVPixelBufferGetBaseAddressOfPlane(imageBuffer, 0)
        let width = CVPixelBufferGetWidthOfPlane(imageBuffer, 0)
        let height = CVPixelBufferGetHeightOfPlane(imageBuffer, 0)
        let stride = CVPixelBufferGetBytesPerRowOfPlane(imageBuffer, 0)
        guard let baseAddress, width > 0, height > 0, stride >= width else { return }
        if !inputFormatValidated {
            inputFormatValidated = true
            let plan = runner.preparationPlan(sourceWidth: width, sourceHeight: height)
            DispatchQueue.main.async { [weak self] in
                self?.cameraReady(
                    sourceWidth: width,
                    sourceHeight: height,
                    sourceStride: stride,
                    plan: plan)
            }
        }
        guard runner.active else { return }
        let length = (height - 1) * stride + width
        let pixelFormat = isVideoRange ? "420v-luma-plane" : "420f-luma-plane"
        runner.processFrame(
            pixels: baseAddress.assumingMemoryBound(to: UInt8.self),
            pixelsLength: length,
            width: width,
            height: height,
            stride: stride,
            pixelFormat: pixelFormat)
    }

    func captureOutput(_ output: AVCaptureOutput, didDrop sampleBuffer: CMSampleBuffer, from connection: AVCaptureConnection) {
        runner?.recordDrop()
    }
}

do {
    let options = try Options(arguments: CommandLine.arguments)
    if options.help {
        print(Options.usage)
    } else if options.fixturePath != nil {
        try runFixture(options)
    } else {
        let application = NSApplication.shared
        let delegate = CameraAppDelegate(options: options)
        application.delegate = delegate
        application.setActivationPolicy(.regular)
        application.run()
    }
} catch {
    fputs("tessipher-camera-bench: \(error)\n", stderr)
    exit(EXIT_FAILURE)
}
