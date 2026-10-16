// Host-only image API comparison. Input loading and payload checks are untimed.
#include "tessipher.h"
#include "quirc.h"
#include "Barcode.h"
#include "ReadBarcode.h"
#ifdef TESSIPHER_MEMORY_PROBE
#include "heap_probe.h"
#endif

#include <algorithm>
#include <array>
#include <charconv>
#include <chrono>
#include <cstdint>
#include <cstring>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <limits>
#include <map>
#include <memory>
#include <stdexcept>
#include <string>
#include <string_view>
#include <vector>

using Clock = std::chrono::steady_clock;
using Bytes = std::vector<uint8_t>;
constexpr size_t kSegments = 64;
constexpr size_t kMaxIterations = 1000000;

struct Observation {
    double latency_us = 0;
    int status = -1;
    bool found = false;
    Bytes payload;
#ifdef TESSIPHER_MEMORY_PROBE
    heap_probe::Snapshot heap{};
#endif
};

#ifdef TESSIPHER_MEMORY_PROBE
struct MemoryMetrics {
    heap_probe::Snapshot setup{}, first{}, decode{};
    size_t caller_fixed_bytes = 0;
    size_t caller_output_bytes = 0, caller_config_bytes = 0, caller_context_bytes = 0;

    void record(const heap_probe::Snapshot &sample, bool first_call) {
        if (first_call)
            first = sample;
        decode.live_bytes = std::max(decode.live_bytes, sample.live_bytes);
        decode.peak_bytes = std::max(decode.peak_bytes, sample.peak_bytes);
        decode.allocation_calls = std::max(decode.allocation_calls, sample.allocation_calls);
        decode.total_allocated_bytes =
            std::max(decode.total_allocated_bytes, sample.total_allocated_bytes);
    }
};
#endif

struct Metrics {
    const char *name;
    std::vector<double> latency;
    size_t correct = 0, misread = 0, not_found = 0;
    std::map<int, size_t> statuses;
#ifdef TESSIPHER_MEMORY_PROBE
    MemoryMetrics memory;
#endif

    explicit Metrics(const char *decoder_name) : name(decoder_name) {}

    void record(const Observation &result, bool negative, const Bytes &expected) {
        latency.push_back(result.latency_us);
        ++statuses[result.status];
        if (result.found) {
            if (!negative && result.payload == expected)
                ++correct;
            else
                ++misread;
        } else if (negative) {
            ++correct;
        } else {
            ++not_found;
        }
    }
};

static size_t number(std::string_view text, size_t minimum, size_t maximum, const char *label) {
    size_t value = 0;
    const auto parsed = std::from_chars(text.data(), text.data() + text.size(), value);
    if (text.empty() || parsed.ec != std::errc{} || parsed.ptr != text.data() + text.size() ||
        value < minimum || value > maximum)
        throw std::runtime_error(std::string("invalid ") + label);
    return value;
}

static Bytes expected_bytes(std::string_view hex) {
    if (hex.size() % 2 || hex.size() > 2 * 7089)
        throw std::runtime_error("invalid expected payload hex length");
    auto digit = [](char c) -> uint8_t {
        if (c >= '0' && c <= '9')
            return static_cast<uint8_t>(c - '0');
        if (c >= 'a' && c <= 'f')
            return static_cast<uint8_t>(c - 'a' + 10);
        if (c >= 'A' && c <= 'F')
            return static_cast<uint8_t>(c - 'A' + 10);
        throw std::runtime_error("invalid expected payload hex digit");
    };
    Bytes result;
    result.reserve(hex.size() / 2);
    for (size_t i = 0; i < hex.size(); i += 2)
        result.push_back(static_cast<uint8_t>((digit(hex[i]) << 4) | digit(hex[i + 1])));
    return result;
}

static double elapsed(Clock::time_point begin) {
    return std::chrono::duration<double, std::micro>(Clock::now() - begin).count();
}

struct TessipherContext {
    Bytes detector, core, payload;
    std::array<qr_segment, kSegments> segments{};
    TessipherContext(const qr_memory_info &memory, size_t detector_bytes)
        : detector(detector_bytes), core(memory.core_scratch_bytes),
          payload(memory.max_payload_bytes) {}

    Observation decode(const qr_config &config, size_t pixel_limit, uint32_t mode,
                       uint32_t strategy, const Bytes &image, size_t width, size_t height) {
        Observation observation;
#ifdef TESSIPHER_MEMORY_PROBE
        heap_probe::begin(strategy == QR_SAMPLING_CENTER ? 0 : 1);
#endif
        qr_result result{};
        const auto begin = Clock::now();
        observation.status = qr_decode_gray_with_mode(
            &config, pixel_limit, mode, strategy, image.data(), image.size(), width, height, width,
            detector.data(), detector.size(), core.data(), core.size(), payload.data(),
            payload.size(), segments.data(), segments.size(), &result, nullptr);
        observation.latency_us = elapsed(begin);
        observation.found = observation.status == QR_OK;
        if (observation.found) {
#ifdef TESSIPHER_MEMORY_PROBE
            heap_probe::Pause pause;
#endif
            observation.payload.assign(payload.begin(), payload.begin() + result.payload_len);
        }
#ifdef TESSIPHER_MEMORY_PROBE
        observation.heap = heap_probe::end();
#endif
        return observation;
    }
};

static Observation decode_quirc(quirc *decoder, const Bytes &image) {
    Observation observation;
#ifdef TESSIPHER_MEMORY_PROBE
    heap_probe::begin(2);
#endif
    quirc_code code{};
    quirc_data data{};
    const auto begin = Clock::now();
    auto *buffer = quirc_begin(decoder, nullptr, nullptr);
    std::memcpy(buffer, image.data(), image.size());
    quirc_end(decoder);
    const int count = quirc_count(decoder);
    // -1 is the harness's no-candidate status; nonnegative values are quirc errors.
    for (int i = 0; i < count; ++i) {
        quirc_extract(decoder, i, &code);
        observation.status = quirc_decode(&code, &data);
        if (observation.status != QUIRC_SUCCESS) {
            quirc_flip(&code);
            observation.status = quirc_decode(&code, &data);
        }
        if (observation.status == QUIRC_SUCCESS) {
            observation.found = true;
            break;
        }
    }
    observation.latency_us = elapsed(begin);
    if (observation.found) {
#ifdef TESSIPHER_MEMORY_PROBE
        heap_probe::Pause pause;
#endif
        observation.payload.assign(data.payload, data.payload + data.payload_len);
    }
#ifdef TESSIPHER_MEMORY_PROBE
    observation.heap = heap_probe::end();
#endif
    return observation;
}

static Observation decode_zxing(const ZXing::ImageView &image,
                                const ZXing::ReaderOptions &options) {
    Observation observation;
#ifdef TESSIPHER_MEMORY_PROBE
    heap_probe::begin(3);
#endif
    {
        ZXing::Barcode barcode;
        const auto begin = Clock::now();
        barcode = ZXing::ReadBarcode(image, options);
        observation.latency_us = elapsed(begin);
        observation.found = barcode.isValid();
        // -1 means no candidate, 0 success, otherwise ZXing::Error::Type's integer.
        observation.status =
            observation.found ? 0
                              : (barcode.error() ? static_cast<int>(barcode.error().type()) : -1);
        if (observation.found) {
#ifdef TESSIPHER_MEMORY_PROBE
            heap_probe::Pause pause;
#endif
            observation.payload = barcode.bytes();
        }
    } // Include the decoder-owned result's destruction in heap accounting.
#ifdef TESSIPHER_MEMORY_PROBE
    observation.heap = heap_probe::end();
#endif
    return observation;
}

#ifdef TESSIPHER_MEMORY_PROBE
static void print_snapshot(const heap_probe::Snapshot &snapshot) {
    std::cout << "{\"live_bytes\":" << snapshot.live_bytes
              << ",\"peak_bytes\":" << snapshot.peak_bytes
              << ",\"allocation_calls\":" << snapshot.allocation_calls
              << ",\"total_allocated_bytes\":" << snapshot.total_allocated_bytes << '}';
}
#endif

static void print_metrics(const Metrics &metrics) {
    std::cout << "{\"name\":\"" << metrics.name << "\",\"latency_us\":[";
    for (size_t i = 0; i < metrics.latency.size(); ++i) {
        if (i)
            std::cout << ',';
        std::cout << metrics.latency[i];
    }
    std::cout << "],\"correct\":" << metrics.correct << ",\"misread\":" << metrics.misread
              << ",\"not_found\":" << metrics.not_found << ",\"statuses\":{";
    bool comma = false;
    for (const auto &[status, count] : metrics.statuses) {
        if (comma)
            std::cout << ',';
        comma = true;
        std::cout << '"' << status << "\":" << count;
    }
    std::cout << '}';
#ifdef TESSIPHER_MEMORY_PROBE
    const auto &memory = metrics.memory;
    std::cout << ",\"memory\":{\"setup_heap\":";
    print_snapshot(memory.setup);
    std::cout << ",\"first_decode_heap\":";
    print_snapshot(memory.first);
    std::cout << ",\"decode_heap\":";
    print_snapshot(memory.decode);
    const size_t peak =
        std::max({memory.setup.peak_bytes, memory.first.peak_bytes, memory.decode.peak_bytes});
    std::cout << ",\"caller_fixed_bytes\":" << memory.caller_fixed_bytes
              << ",\"caller_fixed_breakdown\":{\"output_bytes\":" << memory.caller_output_bytes
              << ",\"config_bytes\":" << memory.caller_config_bytes
              << ",\"context_bytes\":" << memory.caller_context_bytes << '}'
              << ",\"accounted_peak_bytes\":" << peak + memory.caller_fixed_bytes
              << ",\"internal_stack_bytes\":null}";
#endif
    std::cout << '}';
}

int main(int argc, char **argv) {
    try {
        if (argc != 8 && argc != 9)
            throw std::runtime_error("usage: decoder-bench raw_gray width height warmup samples "
                                     "max_pixel_reads expected_hex_or_dash [mode:0=direct,2=fast]");
        const size_t width = number(argv[2], 1, 640, "width");
        const size_t height = number(argv[3], 1, 640, "height");
        const size_t warmup = number(argv[4], 0, kMaxIterations, "warmup");
        const size_t samples = number(argv[5], 1, kMaxIterations, "samples");
        const size_t pixel_limit =
            number(argv[6], 1, std::numeric_limits<size_t>::max(), "pixel budget");
        const bool negative = std::string_view(argv[7]) == "-";
        const Bytes expected = negative ? Bytes{} : expected_bytes(argv[7]);
        const auto mode = static_cast<uint32_t>(argc == 9 ? number(argv[8], 0, 2, "detection mode")
                                                          : QR_DEFAULT_DETECTION_MODE);
        if (mode != QR_MODE_DIRECT && mode != QR_MODE_FAST)
            throw std::runtime_error("invalid detection mode (expected 0 or 2)");
        const char *mode_name = mode == QR_MODE_DIRECT ? "direct" : "fast";
        std::ifstream input(argv[1], std::ios::binary | std::ios::ate);
        if (!input || input.tellg() != static_cast<std::streamoff>(width * height))
            throw std::runtime_error("raw Gray8 file length must equal width * height");
        input.seekg(0);
        Bytes image(width * height);
        if (!input.read(reinterpret_cast<char *>(image.data()),
                        static_cast<std::streamsize>(image.size())))
            throw std::runtime_error("could not read raw Gray8 input");

        qr_config config{};
        qr_memory_info memory{};
        if (qr_default_config(&config) != QR_OK || qr_memory(&config, &memory) != QR_OK)
            throw std::runtime_error("Tessipher configuration/memory query failed");
        if (width > memory.max_width || height > memory.max_height ||
            kSegments > memory.max_segments)
            throw std::runtime_error("input exceeds compiled Tessipher limits");
        size_t detector_bytes = 0;
        if (qr_detector_scratch_bytes(&config, mode, width, height, &detector_bytes) != QR_OK)
            throw std::runtime_error("Tessipher mode scratch query failed");
#ifdef TESSIPHER_MEMORY_PROBE
        heap_probe::begin(0);
#endif
        TessipherContext center(memory, detector_bytes);
#ifdef TESSIPHER_MEMORY_PROBE
        const auto center_setup = heap_probe::end();
        heap_probe::begin(1);
#endif
        TessipherContext majority(memory, detector_bytes);
#ifdef TESSIPHER_MEMORY_PROBE
        const auto majority_setup = heap_probe::end();
        heap_probe::begin(2);
#endif
        std::unique_ptr<quirc, decltype(&quirc_destroy)> quirc_decoder(quirc_new(), &quirc_destroy);
        if (!quirc_decoder || quirc_resize(quirc_decoder.get(), static_cast<int>(width),
                                           static_cast<int>(height)) < 0)
            throw std::runtime_error("quirc allocation failed");
#ifdef TESSIPHER_MEMORY_PROBE
        const auto quirc_setup = heap_probe::end();
        heap_probe::begin(3);
#endif
        ZXing::ReaderOptions options;
        options.setFormats(ZXing::BarcodeFormat::QRCodeModel2)
            .setMaxNumberOfSymbols(1)
            .setTryHarder(true)
            .setTryRotate(true)
            .setTryInvert(true)
            .setTryDownscale(true)
            .setIsPure(false)
            .setBinarizer(ZXing::Binarizer::LocalAverage)
            .setReturnErrors(false);
#ifdef TESSIPHER_MEMORY_PROBE
        const auto zxing_setup = heap_probe::end();
#endif
        const ZXing::ImageView view(image.data(), static_cast<int>(width), static_cast<int>(height),
                                    ZXing::ImageFormat::Lum);
        std::array<Metrics, 4> metrics{{Metrics("tessipher-center"), Metrics("tessipher-majority5"),
                                        Metrics("quirc"), Metrics("zxing-cpp")}};
#ifdef TESSIPHER_MEMORY_PROBE
        metrics[0].memory.setup = center_setup;
        metrics[1].memory.setup = majority_setup;
        metrics[2].memory.setup = quirc_setup;
        metrics[3].memory.setup = zxing_setup;
        const size_t tessipher_fixed =
            sizeof(center.segments) + sizeof(qr_result) + sizeof(config) + 3 * sizeof(Bytes);
        metrics[0].memory.caller_fixed_bytes = tessipher_fixed;
        metrics[1].memory.caller_fixed_bytes = tessipher_fixed;
        metrics[2].memory.caller_fixed_bytes =
            sizeof(quirc_code) + sizeof(quirc_data) + sizeof(quirc_decoder);
        metrics[3].memory.caller_fixed_bytes =
            sizeof(options) + sizeof(view) + sizeof(ZXing::Barcode);
        for (size_t i = 0; i < 2; ++i) {
            metrics[i].memory.caller_output_bytes = sizeof(center.segments) + sizeof(qr_result);
            metrics[i].memory.caller_config_bytes = sizeof(config);
            metrics[i].memory.caller_context_bytes = 3 * sizeof(Bytes);
        }
        metrics[2].memory.caller_output_bytes = sizeof(quirc_code) + sizeof(quirc_data);
        metrics[2].memory.caller_context_bytes = sizeof(quirc_decoder);
        metrics[3].memory.caller_output_bytes = sizeof(ZXing::Barcode);
        metrics[3].memory.caller_config_bytes = sizeof(options);
        metrics[3].memory.caller_context_bytes = sizeof(view);
#endif
        for (auto &metric : metrics)
            metric.latency.reserve(samples);
        for (size_t round = 0; round < warmup + samples; ++round) {
            for (size_t offset = 0; offset < metrics.size(); ++offset) {
                const size_t index = (round + offset) % metrics.size();
                Observation observation;
                switch (index) {
                case 0:
                    observation = center.decode(config, pixel_limit, mode, QR_SAMPLING_CENTER,
                                                image, width, height);
                    break;
                case 1:
                    observation =
                        majority.decode(config, pixel_limit, mode,
                                        QR_SAMPLING_CENTER_THEN_MAJORITY5, image, width, height);
                    break;
                case 2:
                    observation = decode_quirc(quirc_decoder.get(), image);
                    break;
                default:
                    observation = decode_zxing(view, options);
                    break;
                }
#ifdef TESSIPHER_MEMORY_PROBE
                metrics[index].memory.record(observation.heap, round == 0);
#endif
                if (round >= warmup)
                    metrics[index].record(observation, negative, expected);
            }
        }
        std::cout
            << std::setprecision(17) << "{\"tessipher\":{\"abi_version\":" << memory.abi_version
            << ",\"max_width\":" << memory.max_width << ",\"max_height\":" << memory.max_height
            << ",\"detector_scratch_bytes\":" << center.detector.size()
            << ",\"minimum_detector_scratch_bytes\":" << memory.detector_scratch_bytes
            << ",\"core_scratch_bytes\":" << memory.core_scratch_bytes
            << ",\"payload_capacity\":" << memory.max_payload_bytes
            << ",\"segment_capacity\":" << kSegments << ",\"segment_size\":" << memory.segment_size
            << ",\"max_pixel_reads\":" << pixel_limit << ",\"max_finders\":" << config.max_finders
            << ",\"max_candidates\":" << config.max_candidates
            << ",\"max_retries\":" << config.max_retries << ",\"row_step\":" << config.row_step
            << ",\"local_radius\":" << config.local_radius << ",\"detection_mode\":\"" << mode_name
            << "\",\"tracked\":false}"
            << ",\"quirc\":{\"copy_included\":true,\"flip_retry\":true}"
            << ",\"zxing\":{\"format\":\"QRCodeModel2\",\"max_number_of_symbols\":1,\"try_harder\":"
               "true"
            << ",\"try_rotate\":true,\"try_invert\":true,\"try_downscale\":true,\"is_pure\":false"
            << ",\"binarizer\":\"LocalAverage\",\"return_errors\":false,\"downscale_threshold\":"
            << options.downscaleThreshold()
            << ",\"downscale_factor\":" << static_cast<unsigned>(options.downscaleFactor())
            << "},\"decoders\":[";
        for (size_t i = 0; i < metrics.size(); ++i) {
            if (i)
                std::cout << ',';
            print_metrics(metrics[i]);
        }
        std::cout << "]}\n";
        return 0;
    } catch (const std::exception &error) {
        std::cerr << "decoder-bench: " << error.what() << '\n';
        return 1;
    }
}
