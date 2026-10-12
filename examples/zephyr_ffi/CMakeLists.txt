cmake_minimum_required(VERSION 3.20.0)
find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(tessipher_ffi)

if(NOT "${BOARD}/${BOARD_QUALIFIERS}" STREQUAL "mps2/an385")
  message(FATAL_ERROR "This integration is validated only for mps2/an385 (Cortex-M3)")
endif()
get_filename_component(TESSIPHER_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../.." ABSOLUTE)
set(TESSIPHER_RUST_ARCHIVE "" CACHE FILEPATH "thumbv7m-none-eabi no-host release archive")
if(NOT EXISTS "${TESSIPHER_RUST_ARCHIVE}")
  message(FATAL_ERROR "Build Rust first; use tools/zephyr_smoke.py or set TESSIPHER_RUST_ARCHIVE")
endif()
add_library(tessipher_ffi STATIC IMPORTED GLOBAL)
set_target_properties(tessipher_ffi PROPERTIES IMPORTED_LOCATION "${TESSIPHER_RUST_ARCHIVE}")
target_link_libraries(app PRIVATE tessipher_ffi)
target_include_directories(app PRIVATE "${TESSIPHER_ROOT}/include" "${CMAKE_CURRENT_BINARY_DIR}")

# Embed the existing binary fixture at build time, rather than maintaining a copy.
generate_inc_file_for_target(app "${TESSIPHER_ROOT}/fixtures/v1.bin"
                            "${CMAKE_CURRENT_BINARY_DIR}/v1.inc")
target_sources(app PRIVATE src/main.c)
