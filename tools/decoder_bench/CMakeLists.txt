cmake_minimum_required(VERSION 3.24)
project(TessipherDecoderBench LANGUAGES C CXX)

if(NOT IS_ABSOLUTE "${TESSIPHER_ARCHIVE}" OR NOT EXISTS "${TESSIPHER_ARCHIVE}")
  message(FATAL_ERROR "TESSIPHER_ARCHIVE must name an existing absolute-path release archive")
endif()
if(NOT CMAKE_BUILD_TYPE)
  set(CMAKE_BUILD_TYPE Release CACHE STRING "Build type" FORCE)
endif()
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
set(BUILD_SHARED_LIBS OFF CACHE BOOL "Build static libraries" FORCE)

include(FetchContent)
FetchContent_Declare(quirc
  GIT_REPOSITORY https://github.com/dlbeer/quirc.git
  GIT_TAG 542848dd6b9b0eaa9587bbf25b9bc67bd8a71fca)
FetchContent_Declare(zxing
  GIT_REPOSITORY https://github.com/zxing-cpp/zxing-cpp.git
  GIT_TAG 287c85df6f961c8efbfb5ffd736cd9457b8b890e
  GIT_SUBMODULES "")
set(ZXING_READERS ON CACHE BOOL "Build readers" FORCE)
set(ZXING_WRITERS OFF CACHE STRING "Disable writers and their external dependencies" FORCE)
foreach(option C_API EXPERIMENTAL_API EXAMPLES EXAMPLES_QT BLACKBOX_TESTS UNIT_TESTS TEST_DOTNET TEST_GO TEST_INSTALL PYTHON_MODULE)
  set(ZXING_${option} OFF CACHE BOOL "Disabled for the host benchmark" FORCE)
endforeach()
set(ZXING_DEPENDENCIES LOCAL CACHE STRING "Do not fetch optional dependencies" FORCE)
# ZXing's docs project otherwise fetches an unpinned theme when Doxygen is installed.
set(CMAKE_DISABLE_FIND_PACKAGE_Doxygen TRUE)
FetchContent_MakeAvailable(quirc zxing)

# quirc has no CMake project. Build its complete image detection implementation.
set(QUIRC_BENCH_SOURCES
  ${quirc_SOURCE_DIR}/lib/quirc.c
  ${quirc_SOURCE_DIR}/lib/identify.c
  ${quirc_SOURCE_DIR}/lib/decode.c
  ${quirc_SOURCE_DIR}/lib/version_db.c)
add_library(quirc_bench STATIC ${QUIRC_BENCH_SOURCES})
target_include_directories(quirc_bench PUBLIC ${quirc_SOURCE_DIR}/lib)
set_target_properties(quirc_bench PROPERTIES C_STANDARD 99 C_STANDARD_REQUIRED ON)

get_filename_component(TESSIPHER_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../.." ABSOLUTE)
find_package(Threads REQUIRED)
function(configure_benchmark target quirc_target)
  target_include_directories(${target} PRIVATE "${TESSIPHER_ROOT}/include")
  target_link_libraries(${target} PRIVATE "${TESSIPHER_ARCHIVE}" ${quirc_target} ZXing::ZXing)
  target_link_libraries(${target} PRIVATE Threads::Threads ${CMAKE_DL_LIBS})
  if(APPLE)
    target_link_libraries(${target} PRIVATE "-framework Security" "-framework CoreFoundation" iconv)
  elseif(UNIX)
    target_link_libraries(${target} PRIVATE m)
  elseif(WIN32)
    target_link_libraries(${target} PRIVATE ws2_32 userenv bcrypt ntdll)
  endif()
  if(MSVC)
    target_compile_options(${target} PRIVATE /W4)
  else()
    target_compile_options(${target} PRIVATE -Wall -Wextra -Wpedantic)
  endif()
endfunction()

add_executable(decoder-bench main.cpp)
configure_benchmark(decoder-bench quirc_bench)

# Allocation hooks are confined to the separate memory runner. The latency
# runner and its quirc object files retain their ordinary allocation calls.
add_library(quirc_memory STATIC ${QUIRC_BENCH_SOURCES})
target_include_directories(quirc_memory PUBLIC ${quirc_SOURCE_DIR}/lib)
set_target_properties(quirc_memory PROPERTIES C_STANDARD 99 C_STANDARD_REQUIRED ON)
target_compile_definitions(quirc_memory PRIVATE
  malloc=bench_malloc calloc=bench_calloc realloc=bench_realloc free=bench_free)
add_executable(decoder-bench-memory main.cpp heap_probe.cpp)
target_compile_definitions(decoder-bench-memory PRIVATE TESSIPHER_MEMORY_PROBE=1)
configure_benchmark(decoder-bench-memory quirc_memory)

include(CTest)
if(BUILD_TESTING)
  add_executable(heap-probe-test heap_probe.cpp heap_probe_test.cpp)
  target_compile_features(heap-probe-test PRIVATE cxx_std_20)
  add_test(NAME heap-probe-test COMMAND heap-probe-test)
endif()

# The launcher verifies the actual populated sources, including cached overrides.
file(WRITE "${CMAKE_BINARY_DIR}/decoder-sources.txt" "${quirc_SOURCE_DIR}\n${zxing_SOURCE_DIR}\n")
