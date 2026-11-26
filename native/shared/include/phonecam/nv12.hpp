/** NV12 frame scaling, shared by the core and the virtual camera. */
#pragma once

#include <cstdint>

namespace phonecam::nv12 {

/**
 * Scales a tightly packed NV12 frame to another size, by nearest neighbour.
 *
 * Both planes are scaled: the luma plane at full resolution, and the
 * interleaved chroma plane at half, which is what NV12's layout requires.
 *
 * Nearest neighbour is deliberate. It is a copy with no intermediate state, it
 * cannot introduce a colour shift, and the alternative (a Media Foundation video
 * processor) costs a second graph to keep alive for a difference nobody sees.
 *
 * `source` and `destination` are tightly packed: `sourceWidth` bytes per luma
 * row, and the same per chroma row.
 *
 * The destination is `targetWidth * targetHeight * 3 / 2` bytes. The scaler is
 * safe for any inputs, including the odd dimensions NV12 forbids: the last
 * chroma pair of an odd width writes only the byte that exists, so it never
 * touches the byte past the buffer.
 */
void Scale(const uint8_t* source, uint32_t sourceWidth, uint32_t sourceHeight,
           uint8_t* destination, uint32_t targetWidth, uint32_t targetHeight);

/**
 * Applies the display transform (mirror and rotation) to a frame of the same
 * size, letterboxing rather than stretching.
 *
 * The frame keeps its `width` x `height` buffer, and the rotated picture is
 * fitted inside it at the correct aspect ratio; the remainder is filled with
 * black (luma 16, chroma 128). Stretching was the earlier behaviour: a 16:9
 * landscape frame rotated a quarter turn was scaled to fill the same 16:9 box,
 * which distorted it by 3.2x. A phone mounted sideways is the main reason this
 * control exists, so that is the case that has to look right.
 *
 * Mirror is applied in the rotated frame's own space, after the rotation, so a
 * mirrored quarter turn still reads left-to-right the way a mirror does.
 */
void Transform(const uint8_t* source, uint32_t width, uint32_t height, bool mirror,
               uint32_t rotation, uint8_t* destination);

}  // namespace phonecam::nv12
