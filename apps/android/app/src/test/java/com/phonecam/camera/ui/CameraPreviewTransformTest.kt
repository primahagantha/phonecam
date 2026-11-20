package com.phonecam.camera.ui

import com.phonecam.camera.ui.components.calculatePreviewTransform
import com.phonecam.camera.ui.components.previewTransform
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Verifies preview transformations and aspect ratio calculations across
 * portrait and landscape display modes (0°, 90°, 180°, 270°) and sensor orientations.
 *
 * Ensures letterbox/pillarbox black borders and stretching distortion are eliminated
 * when rotating the phone or switching between rear and front cameras.
 */
class CameraPreviewTransformTest {

    private val delta = 0.0001f

    // =========================================================================
    // 1. previewTransform Invocation & Edge Cases
    // =========================================================================

    @Test
    fun `previewTransform returns valid non-null Matrix for normal dimensions`() {
        val matrix = previewTransform(
            sensorOrientation = 90,
            displayRotationDegrees = 0,
            mirror = false,
            viewWidth = 1080,
            viewHeight = 1920,
        )
        assertNotNull(matrix)
    }

    @Test
    fun `previewTransform handles zero and negative dimensions gracefully`() {
        val zeroWidth = previewTransform(90, 0, false, 0, 1920)
        assertNotNull(zeroWidth)

        val zeroHeight = previewTransform(90, 0, false, 1080, 0)
        assertNotNull(zeroHeight)

        val negativeSize = previewTransform(90, 0, false, -1080, -1920)
        assertNotNull(negativeSize)
    }

    @Test
    fun `previewTransform 4-argument overload defaults displayRotationDegrees to 0`() {
        val matrix4 = previewTransform(
            sensorOrientation = 90,
            mirror = false,
            viewWidth = 1080,
            viewHeight = 1920,
        )
        assertNotNull(matrix4)

        val calc0 = calculatePreviewTransform(
            sensorOrientation = 90,
            displayRotationDegrees = 0,
            mirror = false,
            viewWidth = 1080,
            viewHeight = 1920,
        )
        assertEquals(90, calc0.netRotation)
        assertEquals(1920f / 1080f, calc0.scaleX, delta)
        assertEquals(1080f / 1920f, calc0.scaleY, delta)
    }

    // =========================================================================
    // 2. Rear Camera (sensorOrientation = 90°, mirror = false)
    // =========================================================================

    @Test
    fun `rear camera in portrait mode 0 deg applies aspect compensation scale`() {
        val viewWidth = 1080
        val viewHeight = 2400

        val calc = calculatePreviewTransform(
            sensorOrientation = 90,
            displayRotationDegrees = 0,
            mirror = false,
            viewWidth = viewWidth,
            viewHeight = viewHeight,
        )

        // Net rotation: (90 - 0 + 360) % 360 = 90°
        assertEquals(90, calc.netRotation)
        assertFalse(calc.mirror)
        assertEquals(540f, calc.centerX, delta)
        assertEquals(1200f, calc.centerY, delta)

        // Swapping W & H requires scaleX = H/W and scaleY = W/H around center
        assertEquals(viewHeight.toFloat() / viewWidth.toFloat(), calc.scaleX, delta)
        assertEquals(viewWidth.toFloat() / viewHeight.toFloat(), calc.scaleY, delta)
    }

    @Test
    fun `rear camera in landscape mode 90 deg requires no aspect scale`() {
        val viewWidth = 2400
        val viewHeight = 1080

        val calc = calculatePreviewTransform(
            sensorOrientation = 90,
            displayRotationDegrees = 90,
            mirror = false,
            viewWidth = viewWidth,
            viewHeight = viewHeight,
        )

        // Net rotation: (90 - 90 + 360) % 360 = 0°
        assertEquals(0, calc.netRotation)
        assertFalse(calc.mirror)

        // When net rotation is 0, scaling is exactly 1.0 (no distortion)
        assertEquals(1.0f, calc.scaleX, delta)
        assertEquals(1.0f, calc.scaleY, delta)
    }

    @Test
    fun `rear camera in reverse portrait mode 180 deg applies aspect compensation scale`() {
        val viewWidth = 1080
        val viewHeight = 2400

        val calc = calculatePreviewTransform(
            sensorOrientation = 90,
            displayRotationDegrees = 180,
            mirror = false,
            viewWidth = viewWidth,
            viewHeight = viewHeight,
        )

        // Net rotation: (90 - 180 + 360) % 360 = 270°
        assertEquals(270, calc.netRotation)
        assertFalse(calc.mirror)

        assertEquals(viewHeight.toFloat() / viewWidth.toFloat(), calc.scaleX, delta)
        assertEquals(viewWidth.toFloat() / viewHeight.toFloat(), calc.scaleY, delta)
    }

    @Test
    fun `rear camera in reverse landscape mode 270 deg requires no aspect scale`() {
        val viewWidth = 2400
        val viewHeight = 1080

        val calc = calculatePreviewTransform(
            sensorOrientation = 90,
            displayRotationDegrees = 270,
            mirror = false,
            viewWidth = viewWidth,
            viewHeight = viewHeight,
        )

        // Net rotation: (90 - 270 + 360) % 360 = 180°
        assertEquals(180, calc.netRotation)
        assertFalse(calc.mirror)

        // When net rotation is 180, scaling is exactly 1.0
        assertEquals(1.0f, calc.scaleX, delta)
        assertEquals(1.0f, calc.scaleY, delta)
    }

    // =========================================================================
    // 3. Front Camera (sensorOrientation = 270°, mirror = true)
    // =========================================================================

    @Test
    fun `front camera in portrait mode 0 deg applies mirror and aspect compensation`() {
        val viewWidth = 1080
        val viewHeight = 2400

        val calc = calculatePreviewTransform(
            sensorOrientation = 270,
            displayRotationDegrees = 0,
            mirror = true,
            viewWidth = viewWidth,
            viewHeight = viewHeight,
        )

        // Front net rotation: (270 + 0) % 360 = 270°
        assertEquals(270, calc.netRotation)
        assertTrue(calc.mirror)

        assertEquals(viewHeight.toFloat() / viewWidth.toFloat(), calc.scaleX, delta)
        assertEquals(viewWidth.toFloat() / viewHeight.toFloat(), calc.scaleY, delta)
    }

    @Test
    fun `front camera in landscape mode 90 deg mirrors with 1_0 scale`() {
        val viewWidth = 2400
        val viewHeight = 1080

        val calc = calculatePreviewTransform(
            sensorOrientation = 270,
            displayRotationDegrees = 90,
            mirror = true,
            viewWidth = viewWidth,
            viewHeight = viewHeight,
        )

        // Front net rotation: (270 + 90) % 360 = 0°
        assertEquals(0, calc.netRotation)
        assertTrue(calc.mirror)

        assertEquals(1.0f, calc.scaleX, delta)
        assertEquals(1.0f, calc.scaleY, delta)
    }

    @Test
    fun `front camera in reverse portrait mode 180 deg applies aspect scale`() {
        val viewWidth = 1080
        val viewHeight = 2400

        val calc = calculatePreviewTransform(
            sensorOrientation = 270,
            displayRotationDegrees = 180,
            mirror = true,
            viewWidth = viewWidth,
            viewHeight = viewHeight,
        )

        // Front net rotation: (270 + 180) % 360 = 90°
        assertEquals(90, calc.netRotation)
        assertTrue(calc.mirror)

        assertEquals(viewHeight.toFloat() / viewWidth.toFloat(), calc.scaleX, delta)
        assertEquals(viewWidth.toFloat() / viewHeight.toFloat(), calc.scaleY, delta)
    }

    @Test
    fun `front camera in reverse landscape mode 270 deg mirrors with 1_0 scale`() {
        val viewWidth = 2400
        val viewHeight = 1080

        val calc = calculatePreviewTransform(
            sensorOrientation = 270,
            displayRotationDegrees = 270,
            mirror = true,
            viewWidth = viewWidth,
            viewHeight = viewHeight,
        )

        // Front net rotation: (270 + 270) % 360 = 180°
        assertEquals(180, calc.netRotation)
        assertTrue(calc.mirror)

        assertEquals(1.0f, calc.scaleX, delta)
        assertEquals(1.0f, calc.scaleY, delta)
    }

    // =========================================================================
    // 4. Tablet / Native Landscape Sensor (sensorOrientation = 0°, mirror = false)
    // =========================================================================

    @Test
    fun `tablet landscape sensor calculates correct net rotation across all angles`() {
        val w = 1920
        val h = 1080

        val rot0 = calculatePreviewTransform(0, 0, false, w, h)
        assertEquals(0, rot0.netRotation)
        assertEquals(1.0f, rot0.scaleX, delta)

        val rot90 = calculatePreviewTransform(0, 90, false, h, w)
        assertEquals(270, rot90.netRotation)
        assertEquals(w.toFloat() / h.toFloat(), rot90.scaleX, delta)

        val rot180 = calculatePreviewTransform(0, 180, false, w, h)
        assertEquals(180, rot180.netRotation)
        assertEquals(1.0f, rot180.scaleX, delta)

        val rot270 = calculatePreviewTransform(0, 270, false, h, w)
        assertEquals(90, rot270.netRotation)
        assertEquals(w.toFloat() / h.toFloat(), rot270.scaleX, delta)
    }

    // =========================================================================
    // 5. aspectRatioOf Across Portrait and Landscape
    // =========================================================================

    @Test
    fun `aspectRatioOf returns 9_16 in portrait and 16_9 in landscape for 1080p`() {
        val res = "1920x1080"
        val sensorRear = 90

        // Display rotation 0° -> Portrait (9:16)
        val portraitRatio = aspectRatioOf(res, sensorOrientation = sensorRear, displayRotation = 0)
        assertEquals(1080f / 1920f, portraitRatio, delta)
        assertEquals(9f / 16f, portraitRatio, delta)

        // Display rotation 90° -> Landscape (16:9)
        val landscapeRatio = aspectRatioOf(res, sensorOrientation = sensorRear, displayRotation = 90)
        assertEquals(1920f / 1080f, landscapeRatio, delta)
        assertEquals(16f / 9f, landscapeRatio, delta)

        // Display rotation 180° -> Reverse Portrait (9:16)
        val revPortraitRatio = aspectRatioOf(res, sensorOrientation = sensorRear, displayRotation = 180)
        assertEquals(9f / 16f, revPortraitRatio, delta)

        // Display rotation 270° -> Reverse Landscape (16:9)
        val revLandscapeRatio = aspectRatioOf(res, sensorOrientation = sensorRear, displayRotation = 270)
        assertEquals(16f / 9f, revLandscapeRatio, delta)
    }

    @Test
    fun `aspectRatioOf preserves 4_3 aspect ratio for 640x480`() {
        val res = "640x480"
        val sensorRear = 90

        // Portrait: 480 / 640 = 3:4 (0.75)
        val portrait = aspectRatioOf(res, sensorOrientation = sensorRear, displayRotation = 0)
        assertEquals(480f / 640f, portrait, delta)
        assertEquals(0.75f, portrait, delta)

        // Landscape: 640 / 480 = 4:3 (1.3333f)
        val landscape = aspectRatioOf(res, sensorOrientation = sensorRear, displayRotation = 90)
        assertEquals(640f / 480f, landscape, delta)
    }

    @Test
    fun `aspectRatioOf handles 4K 3840x2160 correctly`() {
        val res = "3840x2160"
        val sensorRear = 90

        val portrait = aspectRatioOf(res, sensorOrientation = sensorRear, displayRotation = 0)
        assertEquals(2160f / 3840f, portrait, delta)
        assertEquals(9f / 16f, portrait, delta)

        val landscape = aspectRatioOf(res, sensorOrientation = sensorRear, displayRotation = 90)
        assertEquals(3840f / 2160f, landscape, delta)
        assertEquals(16f / 9f, landscape, delta)
    }

    @Test
    fun `aspectRatioOf normalizes inverted resolution strings`() {
        // Even if reported as "1080x1920" (height x width swapped)
        val invertedRes = "1080x1920"
        val sensorRear = 90

        val portrait = aspectRatioOf(invertedRes, sensorOrientation = sensorRear, displayRotation = 0)
        assertEquals(9f / 16f, portrait, delta)

        val landscape = aspectRatioOf(invertedRes, sensorOrientation = sensorRear, displayRotation = 90)
        assertEquals(16f / 9f, landscape, delta)
    }

    @Test
    fun `aspectRatioOf handles null and invalid resolution gracefully`() {
        val sensorRear = 90

        // null resolution
        assertEquals(9f / 16f, aspectRatioOf(null, sensorRear, 0), delta)
        assertEquals(16f / 9f, aspectRatioOf(null, sensorRear, 90), delta)

        // empty string
        assertEquals(9f / 16f, aspectRatioOf("", sensorRear, 0), delta)
        assertEquals(16f / 9f, aspectRatioOf("", sensorRear, 90), delta)

        // invalid string
        assertEquals(9f / 16f, aspectRatioOf("invalid", sensorRear, 0), delta)
        assertEquals(16f / 9f, aspectRatioOf("abc x def", sensorRear, 90), delta)

        // zero or negative values
        assertEquals(9f / 16f, aspectRatioOf("0x0", sensorRear, 0), delta)
        assertEquals(16f / 9f, aspectRatioOf("-1920x1080", sensorRear, 90), delta)
    }
}
