/**
 * Generates NV12 placeholder frames and test patterns for the PhoneCam virtual camera.
 *
 * Used when:
 * 1. The phone is disconnected, stale, or reconnecting (PRD §16).
 * 2. PHONECAM_VCAM_TEST_PATTERN=1 is set for offline testing without hardware.
 * 3. Before the first live frame has been received from the shared memory bus.
 *
 * The disconnected frame carries **readable text**, because the consumer is a
 * video application showing it to a person: an unlabelled rectangle reads as a
 * broken camera, while "Device not connected" reads as a state. Text is drawn
 * with a built-in 5x7 bitmap font so this stays dependency-free — pulling in
 * DirectWrite or GDI would mean initialising a graphics stack inside the Frame
 * Server process just to draw two lines.
 */

#pragma once

#include <cstdint>
#include <cstring>
#include <vector>
#include <algorithm>

namespace phonecam::vcam {

struct YuvColor {
  uint8_t y;
  uint8_t u;
  uint8_t v;
};

// Standard Rec.601 color bar values
inline constexpr YuvColor kColorWhite   = {235, 128, 128};
inline constexpr YuvColor kColorYellow  = {210,  16, 146};
inline constexpr YuvColor kColorCyan    = {170, 166,  16};
inline constexpr YuvColor kColorGreen   = {145,  54,  34};
inline constexpr YuvColor kColorMagenta = {106, 202, 222};
inline constexpr YuvColor kColorRed     = { 81,  90, 240};
inline constexpr YuvColor kColorBlue    = { 41, 240, 110};
inline constexpr YuvColor kColorBlack   = { 16, 128, 128};

// Disconnected placeholder palette. Neutral chroma throughout: a tinted
// background looks like a colour-cast bug, and the point of this frame is to
// look deliberate.
inline constexpr YuvColor kColorBackground = { 22, 128, 128};  // near-black
inline constexpr YuvColor kColorCardBorder = { 48, 128, 128};  // faint outline
inline constexpr YuvColor kColorTextBright = {235, 128, 128};  // white
inline constexpr YuvColor kColorTextDim    = {140, 128, 128};  // grey

/**
 * A 5x7 bitmap font, uppercase plus the punctuation the placeholders need.
 *
 * Each glyph is five column bytes; bit 0 is the top row. Only the characters
 * actually used are included, so there is no unused table to maintain.
 */
namespace font5x7 {

struct Glyph {
  char character;
  uint8_t columns[5];
};

inline constexpr Glyph kGlyphs[] = {
    {' ', {0x00, 0x00, 0x00, 0x00, 0x00}},
    {'-', {0x08, 0x08, 0x08, 0x08, 0x08}},
    {'.', {0x00, 0x60, 0x60, 0x00, 0x00}},
    {':', {0x00, 0x36, 0x36, 0x00, 0x00}},
    {'0', {0x3E, 0x51, 0x49, 0x45, 0x3E}},
    {'1', {0x00, 0x42, 0x7F, 0x40, 0x00}},
    {'2', {0x42, 0x61, 0x51, 0x49, 0x46}},
    {'3', {0x21, 0x41, 0x45, 0x4B, 0x31}},
    {'4', {0x18, 0x14, 0x12, 0x7F, 0x10}},
    {'5', {0x27, 0x45, 0x45, 0x45, 0x39}},
    {'6', {0x3C, 0x4A, 0x49, 0x49, 0x30}},
    {'7', {0x01, 0x71, 0x09, 0x05, 0x03}},
    {'8', {0x36, 0x49, 0x49, 0x49, 0x36}},
    {'9', {0x06, 0x49, 0x49, 0x29, 0x1E}},
    {'A', {0x7E, 0x11, 0x11, 0x11, 0x7E}},
    {'B', {0x7F, 0x49, 0x49, 0x49, 0x36}},
    {'C', {0x3E, 0x41, 0x41, 0x41, 0x22}},
    {'D', {0x7F, 0x41, 0x41, 0x22, 0x1C}},
    {'E', {0x7F, 0x49, 0x49, 0x49, 0x41}},
    {'F', {0x7F, 0x09, 0x09, 0x09, 0x01}},
    {'G', {0x3E, 0x41, 0x49, 0x49, 0x7A}},
    {'H', {0x7F, 0x08, 0x08, 0x08, 0x7F}},
    {'I', {0x00, 0x41, 0x7F, 0x41, 0x00}},
    {'J', {0x20, 0x40, 0x41, 0x3F, 0x01}},
    {'K', {0x7F, 0x08, 0x14, 0x22, 0x41}},
    {'L', {0x7F, 0x40, 0x40, 0x40, 0x40}},
    {'M', {0x7F, 0x02, 0x0C, 0x02, 0x7F}},
    {'N', {0x7F, 0x04, 0x08, 0x10, 0x7F}},
    {'O', {0x3E, 0x41, 0x41, 0x41, 0x3E}},
    {'P', {0x7F, 0x09, 0x09, 0x09, 0x06}},
    {'Q', {0x3E, 0x41, 0x51, 0x21, 0x5E}},
    {'R', {0x7F, 0x09, 0x19, 0x29, 0x46}},
    {'S', {0x46, 0x49, 0x49, 0x49, 0x31}},
    {'T', {0x01, 0x01, 0x7F, 0x01, 0x01}},
    {'U', {0x3F, 0x40, 0x40, 0x40, 0x3F}},
    {'V', {0x1F, 0x20, 0x40, 0x20, 0x1F}},
    {'W', {0x3F, 0x40, 0x38, 0x40, 0x3F}},
    {'X', {0x63, 0x14, 0x08, 0x14, 0x63}},
    {'Y', {0x07, 0x08, 0x70, 0x08, 0x07}},
    {'Z', {0x61, 0x51, 0x49, 0x45, 0x43}},
};

inline const uint8_t* Find(char character) {
  // Uppercase the input so callers can pass mixed-case strings.
  if (character >= 'a' && character <= 'z') character = static_cast<char>(character - 'a' + 'A');
  for (const Glyph& glyph : kGlyphs) {
    if (glyph.character == character) return glyph.columns;
  }
  return kGlyphs[0].columns;  // space
}

}  // namespace font5x7

/// Width in pixels of `text` drawn at `scale`, with one blank column between glyphs.
inline uint32_t TextWidth(const char* text, uint32_t scale) {
  const uint32_t length = static_cast<uint32_t>(std::strlen(text));
  if (length == 0) return 0;
  // 5 columns plus 1 spacing per glyph, minus the trailing spacing.
  return (length * 6 - 1) * scale;
}

inline uint32_t TextHeight(uint32_t scale) { return 7 * scale; }

/**
 * Draws `text` into the Y plane, writing neutral chroma so the glyphs are
 * white/grey rather than tinted by whatever is behind them.
 */
inline void DrawText(uint8_t* yPlane, uint8_t* uvPlane, uint32_t width, uint32_t height,
                     uint32_t x, uint32_t y, uint32_t scale, const char* text, YuvColor color) {
  uint32_t cursor = x;

  for (const char* p = text; *p != '\0'; ++p) {
    const uint8_t* columns = font5x7::Find(*p);

    for (uint32_t column = 0; column < 5; ++column) {
      const uint8_t bits = columns[column];
      for (uint32_t row = 0; row < 7; ++row) {
        if ((bits & (1u << row)) == 0) continue;

        for (uint32_t dy = 0; dy < scale; ++dy) {
          for (uint32_t dx = 0; dx < scale; ++dx) {
            const uint32_t px = cursor + column * scale + dx;
            const uint32_t py = y + row * scale + dy;
            if (px >= width || py >= height) continue;

            yPlane[static_cast<size_t>(py) * width + px] = color.y;
            // NV12 chroma is subsampled 2x2, so one sample covers this pixel.
            const size_t uvIndex = static_cast<size_t>(py / 2) * width + (px / 2) * 2;
            uvPlane[uvIndex + 0] = color.u;
            uvPlane[uvIndex + 1] = color.v;
          }
        }
      }
    }

    cursor += 6 * scale;
  }
}

/// Draws `text` horizontally centred on a `width`-wide frame.
inline void DrawTextCentered(uint8_t* yPlane, uint8_t* uvPlane, uint32_t width, uint32_t height,
                             uint32_t y, uint32_t scale, const char* text, YuvColor color) {
  const uint32_t textWidth = TextWidth(text, scale);
  const uint32_t x = textWidth < width ? (width - textWidth) / 2 : 0;
  DrawText(yPlane, uvPlane, width, height, x, y, scale, text, color);
}

/**
 * Fills an NV12 buffer with SMPTE-style 8-color test bars.
 * Width and height must be even numbers.
 */
inline void GenerateColorBarsNV12(uint32_t width, uint32_t height, std::vector<uint8_t>* out) {
  const size_t ySize = static_cast<size_t>(width) * height;
  const size_t uvSize = ySize / 2;
  out->resize(ySize + uvSize);

  uint8_t* yPlane = out->data();
  uint8_t* uvPlane = out->data() + ySize;

  const YuvColor bars[8] = {
      kColorWhite, kColorYellow, kColorCyan, kColorGreen,
      kColorMagenta, kColorRed, kColorBlue, kColorBlack
  };

  const uint32_t barWidth = width / 8;

  for (uint32_t r = 0; r < height; ++r) {
    uint8_t* rowY = yPlane + static_cast<size_t>(r) * width;
    for (uint32_t c = 0; c < width; ++c) {
      uint32_t barIdx = std::min<uint32_t>(c / barWidth, 7);
      rowY[c] = bars[barIdx].y;
    }
  }

  const uint32_t uvHeight = height / 2;
  const uint32_t uvWidth = width / 2;
  for (uint32_t r = 0; r < uvHeight; ++r) {
    uint8_t* rowUv = uvPlane + static_cast<size_t>(r) * width;
    for (uint32_t c = 0; c < uvWidth; ++c) {
      uint32_t barIdx = std::min<uint32_t>((c * 2) / barWidth, 7);
      rowUv[c * 2 + 0] = bars[barIdx].u;
      rowUv[c * 2 + 1] = bars[barIdx].v;
    }
  }
}

/**
 * Fills an NV12 buffer with the "device not connected" frame.
 *
 * The text is what makes this useful: a consumer showing this frame to a person
 * must say *why* there is no picture. It also names the fix, because the frame
 * may be the only thing the user is looking at.
 */
inline void GenerateDisconnectedNV12(uint32_t width, uint32_t height, std::vector<uint8_t>* out) {
  const size_t ySize = static_cast<size_t>(width) * height;
  const size_t uvSize = ySize / 2;
  out->resize(ySize + uvSize);

  uint8_t* yPlane = out->data();
  uint8_t* uvPlane = out->data() + ySize;

  std::memset(yPlane, kColorBackground.y, ySize);
  std::memset(uvPlane, 128, uvSize);

  // Scale the type to the frame so the message is legible at every supported
  // resolution instead of only at 1080p.
  const uint32_t scale = std::max<uint32_t>(1, height / 260);
  const uint32_t titleHeight = TextHeight(scale * 2);
  const uint32_t lineHeight = TextHeight(scale);
  const uint32_t gap = scale * 10;

  const uint32_t blockHeight = titleHeight + gap + lineHeight + gap / 2 + lineHeight;
  uint32_t y = height > blockHeight ? (height - blockHeight) / 2 : 0;

  DrawTextCentered(yPlane, uvPlane, width, height, y, scale * 2, "PHONECAM", kColorTextBright);
  y += titleHeight + gap;
  DrawTextCentered(yPlane, uvPlane, width, height, y, scale, "DEVICE NOT CONNECTED", kColorTextBright);
  y += lineHeight + gap / 2;
  DrawTextCentered(yPlane, uvPlane, width, height, y, scale,
                   "OPEN PHONECAM ON YOUR PHONE", kColorTextDim);
}

}  // namespace phonecam::vcam
