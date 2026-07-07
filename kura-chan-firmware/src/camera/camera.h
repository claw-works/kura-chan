#pragma once

#include <stddef.h>

// M5Stack CoreS3 built-in camera (GC0308, 640x480 VGA).
namespace cam {

// Capture one photo and return it as a NUL-terminated JPEG base64 string.
// The buffer is allocated in PSRAM; the caller MUST free() it.
// On success returns the pointer and writes its length (excluding NUL) to
// *out_len. Returns nullptr on any failure.
char* capture_jpeg_base64(size_t* out_len);

} // namespace cam
