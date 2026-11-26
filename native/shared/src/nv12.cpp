#include "phonecam/nv12.hpp"

#include <cstddef>
#include <cstring>

namespace phonecam::nv12 {

void Scale(const uint8_t* source, uint32_t sourceWidth, uint32_t sourceHeight,
           uint8_t* destination, uint32_t targetWidth, uint32_t targetHeight) {
  if (source == nullptr || destination == nullptr) return;
  if (sourceWidth == 0 || sourceHeight == 0 || targetWidth == 0 || targetHeight == 0) return;

  const uint32_t sourceStride = sourceWidth;
  const uint32_t targetStride = targetWidth;

  for (uint32_t y = 0; y < targetHeight; ++y) {
    const uint32_t sourceY =
        static_cast<uint32_t>(static_cast<uint64_t>(y) * sourceHeight / targetHeight);
    const uint8_t* row = source + static_cast<size_t>(sourceY) * sourceStride;
    uint8_t* out = destination + static_cast<size_t>(y) * targetStride;
    for (uint32_t x = 0; x < targetWidth; ++x) {
      out[x] = row[static_cast<uint32_t>(static_cast<uint64_t>(x) * sourceWidth / targetWidth)];
    }
  }

  const uint8_t* sourceChroma = source + static_cast<size_t>(sourceStride) * sourceHeight;
  uint8_t* targetChroma = destination + static_cast<size_t>(targetStride) * targetHeight;
  const uint32_t targetChromaHeight = targetHeight / 2;
  const uint32_t sourceChromaHeight = sourceHeight / 2;
  const uint32_t targetChromaWidth = targetWidth;  // two interleaved bytes per pair
  const uint32_t sourceChromaWidth = sourceWidth;

  for (uint32_t y = 0; y < targetChromaHeight; ++y) {
    const uint32_t sourceY = static_cast<uint32_t>(
        static_cast<uint64_t>(y) * sourceChromaHeight / targetChromaHeight);
    const uint8_t* row = sourceChroma + static_cast<size_t>(sourceY) * sourceStride;
    uint8_t* out = targetChroma + static_cast<size_t>(y) * targetStride;
    for (uint32_t x = 0; x < targetChromaWidth; x += 2) {
      // Chroma is interleaved in pairs, so both bytes come from the same pair.
      const uint32_t sourceX = static_cast<uint32_t>(
                                   static_cast<uint64_t>(x) * sourceChromaWidth / targetChromaWidth) &
                               ~1u;
      out[x] = row[sourceX];
      // A pair is two bytes, but an odd target width ends the last pair on its
      // first byte: `x == targetChromaWidth - 1`, so writing `x + 1` would land
      // on index `targetChromaWidth`, one past the last byte of the last chroma
      // row — and one past the caller's `w*h*3/2` allocation. NV12 requires even
      // dimensions, so this is a malformed input rather than a real frame, but
      // the scaler must still be safe for any input it is handed: write the
      // second byte only when it exists, keeping both bytes of a pair from the
      // same source pair.
      if (x + 1 < targetChromaWidth) {
        out[x + 1] = row[sourceX + 1 < sourceChromaWidth ? sourceX + 1 : sourceX];
      }
    }
  }
}

void Transform(const uint8_t* source, uint32_t width, uint32_t height, bool mirror,
               uint32_t rotation, uint8_t* destination) {
  if (source == nullptr || destination == nullptr || width == 0 || height == 0) return;
  const uint32_t angle = ((rotation % 360u) + 360u) % 360u;
  const bool quarterTurn = angle == 90u || angle == 270u;

  // Black in NV12: luma is limited-range 16, chroma is neutral 128.
  const size_t lumaBytes = static_cast<size_t>(width) * height;
  std::memset(destination, 16, lumaBytes);
  std::memset(destination + lumaBytes, 128, lumaBytes / 2);

  // The rotated picture has swapped dimensions; fit it inside the frame without
  // changing its aspect ratio.
  const uint32_t rotatedWidth = quarterTurn ? height : width;
  const uint32_t rotatedHeight = quarterTurn ? width : height;
  const uint64_t scaleByWidth = static_cast<uint64_t>(width) * rotatedHeight;
  const uint64_t scaleByHeight = static_cast<uint64_t>(height) * rotatedWidth;
  uint32_t fitWidth = 0;
  uint32_t fitHeight = 0;
  if (scaleByWidth <= scaleByHeight) {
    fitWidth = width;
    fitHeight = static_cast<uint32_t>(scaleByWidth / rotatedWidth);
  } else {
    fitHeight = height;
    fitWidth = static_cast<uint32_t>(scaleByHeight / rotatedHeight);
  }
  if (fitWidth == 0 || fitHeight == 0) return;
  // Even dimensions keep the chroma pairs aligned. Rounding to even can zero a
  // dimension when the rotated picture is much wider than the frame (a 4x2 frame
  // turned a quarter turn fits as 1x2, and 1 rounds to 0), which would leave the
  // frame entirely black. The floor of 2 keeps the picture on screen.
  fitWidth &= ~1u;
  fitHeight &= ~1u;
  if (fitWidth == 0 && width >= 2u) fitWidth = 2u;
  if (fitHeight == 0 && height >= 2u) fitHeight = 2u;
  if (fitWidth == 0 || fitHeight == 0) return;
  if (fitWidth > width || fitHeight > height) return;

  const uint32_t offsetX = (width - fitWidth) / 2u;
  const uint32_t offsetY = (height - fitHeight) / 2u;
  const uint8_t* sourceUv = source + lumaBytes;
  uint8_t* destinationUv = destination + lumaBytes;

  // Maps a point in the fitted, rotated picture back to the source.
  //
  // The four cases are the inverse of the renderer's own draw call, which is
  // `translate(centre)`, `rotate(θ)`, `scale(-1,1)`, `drawImage`. Because canvas
  // composes right-to-left, that applies the mirror to the frame's own
  // coordinates *first* and then rotates; the equivalent here is to rotate and
  // then flip the source's horizontal axis, which is why the mirror is the last
  // step rather than part of the case. Flipping before the rotation instead
  // mirrors along the sensor's axis, and the preview and the camera then disagree
  // for every non-zero rotation: for a quarter turn the preview flips the output
  // vertically while a pre-rotation flip would flip it horizontally.
  auto sourcePoint = [&](uint32_t x, uint32_t y, uint32_t* sourceX, uint32_t* sourceY) {
    // Normalise to the rotated picture's own coordinates first.
    const uint32_t rx =
        static_cast<uint32_t>(static_cast<uint64_t>(x) * rotatedWidth / fitWidth);
    const uint32_t ry =
        static_cast<uint32_t>(static_cast<uint64_t>(y) * rotatedHeight / fitHeight);
    uint32_t sx = 0;
    uint32_t sy = 0;
    switch (angle) {
      case 90:
        // Clockwise: the rotated picture's top-left comes from the source's
        // bottom-left.
        sx = ry;
        sy = height - 1u - rx;
        break;
      case 180:
        sx = width - 1u - rx;
        sy = height - 1u - ry;
        break;
      case 270:
        sx = width - 1u - ry;
        sy = rx;
        break;
      default:
        sx = rx;
        sy = ry;
        break;
    }
    if (mirror) sx = width - 1u - sx;
    *sourceX = sx < width ? sx : width - 1u;
    *sourceY = sy < height ? sy : height - 1u;
  };

  for (uint32_t y = 0; y < fitHeight; ++y) {
    uint8_t* out = destination + static_cast<size_t>(y + offsetY) * width + offsetX;
    for (uint32_t x = 0; x < fitWidth; ++x) {
      uint32_t sx = 0;
      uint32_t sy = 0;
      sourcePoint(x, y, &sx, &sy);
      out[x] = source[static_cast<size_t>(sy) * width + sx];
    }
  }

  for (uint32_t y = 0; y < fitHeight / 2u; ++y) {
    uint8_t* out = destinationUv + static_cast<size_t>(y + offsetY / 2u) * width + offsetX;
    for (uint32_t x = 0; x < fitWidth; x += 2u) {
      uint32_t sx = 0;
      uint32_t sy = 0;
      sourcePoint(x, y * 2u, &sx, &sy);
      // Chroma is subsampled in both directions and interleaved in pairs, so the
      // source point is snapped to the even pair the two bytes share.
      const uint32_t pairX = sx & ~1u;
      const uint32_t pairY = sy & ~1u;
      const size_t sourceOffset = static_cast<size_t>(pairY / 2u) * width + pairX;
      out[x] = sourceUv[sourceOffset];
      out[x + 1] = sourceUv[sourceOffset + 1u];
    }
  }
}

}  // namespace phonecam::nv12
