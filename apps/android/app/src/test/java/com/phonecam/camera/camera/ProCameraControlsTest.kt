package com.phonecam.camera.camera

import com.phonecam.camera.protocol.ControlCodec
import com.phonecam.camera.protocol.DesktopMessage
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ProCameraControlsTest {

    @Test
    fun `kelvinToRggb computes correct color gains across temperature spectrum`() {
        // Warm temperature (2800K - tungsten): needs blue boost, lower red
        val warmGains = CameraController.kelvinToColorGains(2800)
        assertEquals(1.0f, warmGains.greenEven, 0.001f)
        assertEquals(1.0f, warmGains.greenOdd, 0.001f)
        assertTrue("Warm light requires blue gain > red gain", warmGains.blue > warmGains.red)

        // Cool temperature (7500K - shade): needs red boost, lower blue
        val coolGains = CameraController.kelvinToColorGains(7500)
        assertEquals(1.0f, coolGains.greenEven, 0.001f)
        assertEquals(1.0f, coolGains.greenOdd, 0.001f)
        assertTrue("Cool light requires red gain > blue gain", coolGains.red > coolGains.blue)

        // Extreme bounds clamping
        val minGains = CameraController.kelvinToColorGains(1000)
        val clamped2000 = CameraController.kelvinToColorGains(2000)
        assertEquals(clamped2000.red, minGains.red, 0.001f)
        assertEquals(clamped2000.blue, minGains.blue, 0.001f)

        val maxGains = CameraController.kelvinToColorGains(15000)
        val clamped10000 = CameraController.kelvinToColorGains(10000)
        assertEquals(clamped10000.red, maxGains.red, 0.001f)
        assertEquals(clamped10000.blue, maxGains.blue, 0.001f)
    }

    @Test
    fun `lens types classify correctly with expected defaults`() {
        assertEquals(0.5f, LensType.ULTRA_WIDE.defaultZoom, 0.001f)
        assertEquals(1.0f, LensType.WIDE.defaultZoom, 0.001f)
        assertEquals(2.0f, LensType.TELEPHOTO.defaultZoom, 0.001f)
        assertEquals(1.0f, LensType.FRONT.defaultZoom, 0.001f)

        val ultraWide = PhysicalLens(
            id = "2",
            type = LensType.ULTRA_WIDE,
            focalLengthMm = 1.8f,
            focalLength35mmEquiv = 14f,
        )
        assertEquals("Ultra-Wide", ultraWide.type.displayName)
        assertEquals(0.5f, ultraWide.zoomFactor, 0.001f)
        assertTrue(!ultraWide.isFront)

        val tele = PhysicalLens(
            id = "3",
            type = LensType.TELEPHOTO,
            focalLengthMm = 6.0f,
            focalLength35mmEquiv = 65f,
        )
        assertEquals("Telephoto", tele.type.displayName)
        assertEquals(2.0f, tele.zoomFactor, 0.001f)
    }

    @Test
    fun `desktop control messages for pro controls decode properly`() {
        val exposurePayload = """{"t":"set_exposure","compensation":-2}""".toByteArray(Charsets.UTF_8)
        val exposureMsg = ControlCodec.decodeDesktop(exposurePayload)
        assertNotNull(exposureMsg)
        assertTrue(exposureMsg is DesktopMessage.SetExposure)
        assertEquals(-2, (exposureMsg as DesktopMessage.SetExposure).compensation)

        val wbPayload = """{"t":"set_white_balance","kelvin":4500}""".toByteArray(Charsets.UTF_8)
        val wbMsg = ControlCodec.decodeDesktop(wbPayload)
        assertNotNull(wbMsg)
        assertTrue(wbMsg is DesktopMessage.SetWhiteBalance)
        assertEquals(4500, (wbMsg as DesktopMessage.SetWhiteBalance).kelvin)

        val wbAutoPayload = """{"t":"set_white_balance"}""".toByteArray(Charsets.UTF_8)
        val wbAutoMsg = ControlCodec.decodeDesktop(wbAutoPayload)
        assertNotNull(wbAutoMsg)
        assertTrue(wbAutoMsg is DesktopMessage.SetWhiteBalance)
        assertNull((wbAutoMsg as DesktopMessage.SetWhiteBalance).kelvin)

        val focusPayload = """{"t":"set_focus","distance":2.5}""".toByteArray(Charsets.UTF_8)
        val focusMsg = ControlCodec.decodeDesktop(focusPayload)
        assertNotNull(focusMsg)
        assertTrue(focusMsg is DesktopMessage.SetFocus)
        assertEquals(2.5f, (focusMsg as DesktopMessage.SetFocus).distance!!, 0.001f)

        val focusAutoPayload = """{"t":"set_focus"}""".toByteArray(Charsets.UTF_8)
        val focusAutoMsg = ControlCodec.decodeDesktop(focusAutoPayload)
        assertNotNull(focusAutoMsg)
        assertTrue(focusAutoMsg is DesktopMessage.SetFocus)
        assertNull((focusAutoMsg as DesktopMessage.SetFocus).distance)
    }

    @Test
    fun `desktop control messages for audio stream and anti flicker decode properly`() {
        val audioOnPayload = """{"t":"set_audio_stream","enabled":true}""".toByteArray(Charsets.UTF_8)
        val audioOnMsg = ControlCodec.decodeDesktop(audioOnPayload)
        assertNotNull(audioOnMsg)
        assertTrue(audioOnMsg is DesktopMessage.SetAudioStream)
        assertTrue((audioOnMsg as DesktopMessage.SetAudioStream).enabled)

        val audioOffPayload = """{"t":"set_audio_stream","enabled":false}""".toByteArray(Charsets.UTF_8)
        val audioOffMsg = ControlCodec.decodeDesktop(audioOffPayload)
        assertNotNull(audioOffMsg)
        assertTrue(audioOffMsg is DesktopMessage.SetAudioStream)
        assertEquals(false, (audioOffMsg as DesktopMessage.SetAudioStream).enabled)

        val antiFlickerPayload = """{"t":"set_anti_flicker","mode":"60hz"}""".toByteArray(Charsets.UTF_8)
        val antiFlickerMsg = ControlCodec.decodeDesktop(antiFlickerPayload)
        assertNotNull(antiFlickerMsg)
        assertTrue(antiFlickerMsg is DesktopMessage.SetAntiFlicker)
        assertEquals("60hz", (antiFlickerMsg as DesktopMessage.SetAntiFlicker).mode)
    }

    @Test
    fun `selectFpsRangeFromPairs selects range with upper == 60 when 60 fps is requested`() {
        // Device with [60, 60] and [30, 60]
        val rangesWithBoth = listOf(
            15 to 30,
            30 to 30,
            30 to 60,
            60 to 60,
        )
        val selected60 = CameraController.selectFpsRangeFromPairs(rangesWithBoth, 60)
        assertEquals(60, selected60.second)
        assertEquals(60, selected60.first)

        // Device with only [30, 60]
        val rangesWith3060 = listOf(
            15 to 30,
            30 to 30,
            30 to 60,
        )
        val selected3060 = CameraController.selectFpsRangeFromPairs(rangesWith3060, 60)
        assertEquals(60, selected3060.second)
        assertEquals(30, selected3060.first)

        // Device with 30fps max: must pick fixed [30, 30] over variable [15, 30]
        val ranges30Max = listOf(
            15 to 30,
            30 to 30,
        )
        val selected30 = CameraController.selectFpsRangeFromPairs(ranges30Max, 30)
        assertEquals(30, selected30.second)
        assertEquals(30, selected30.first)

        // Device with only variable 30fps ranges: falls back to highest lower bound
        val rangesOnlyVariable = listOf(
            15 to 30,
            24 to 30,
        )
        val selectedVariable = CameraController.selectFpsRangeFromPairs(rangesOnlyVariable, 30)
        assertEquals(30, selectedVariable.second)
        assertEquals(24, selectedVariable.first)
    }

    @Test
    fun `resolveAntibandingModeFallback maps modes correctly`() {
        assertEquals(
            android.hardware.camera2.CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_50HZ,
            CameraController.resolveAntibandingModeFallback("50hz"),
        )
        assertEquals(
            android.hardware.camera2.CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_50HZ,
            CameraController.resolveAntibandingModeFallback("50"),
        )
        assertEquals(
            android.hardware.camera2.CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_60HZ,
            CameraController.resolveAntibandingModeFallback("60hz"),
        )
        assertEquals(
            android.hardware.camera2.CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_60HZ,
            CameraController.resolveAntibandingModeFallback("60"),
        )
        assertEquals(
            android.hardware.camera2.CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_OFF,
            CameraController.resolveAntibandingModeFallback("off"),
        )
        assertEquals(
            android.hardware.camera2.CaptureRequest.CONTROL_AE_ANTIBANDING_MODE_AUTO,
            CameraController.resolveAntibandingModeFallback("auto"),
        )
    }

    @Test
    fun `camera controller max video dimensions support 4K`() {
        assertTrue(CameraController.MAX_VIDEO_WIDTH >= 3840)
        assertTrue(CameraController.MAX_VIDEO_HEIGHT >= 2160)
    }

    @Test
    fun `selectAfModeFromModes prioritizes CONTINUOUS_PICTURE over CONTINUOUS_VIDEO, AUTO, and OFF`() {
        val allModes = intArrayOf(
            android.hardware.camera2.CaptureRequest.CONTROL_AF_MODE_OFF,
            android.hardware.camera2.CaptureRequest.CONTROL_AF_MODE_AUTO,
            android.hardware.camera2.CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO,
            android.hardware.camera2.CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE,
        )
        assertEquals(
            android.hardware.camera2.CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE,
            CameraController.selectAfModeFromModes(allModes),
        )

        val videoAndAuto = intArrayOf(
            android.hardware.camera2.CaptureRequest.CONTROL_AF_MODE_OFF,
            android.hardware.camera2.CaptureRequest.CONTROL_AF_MODE_AUTO,
            android.hardware.camera2.CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO,
        )
        assertEquals(
            android.hardware.camera2.CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO,
            CameraController.selectAfModeFromModes(videoAndAuto),
        )

        val autoOnly = intArrayOf(
            android.hardware.camera2.CaptureRequest.CONTROL_AF_MODE_OFF,
            android.hardware.camera2.CaptureRequest.CONTROL_AF_MODE_AUTO,
        )
        assertEquals(
            android.hardware.camera2.CaptureRequest.CONTROL_AF_MODE_AUTO,
            CameraController.selectAfModeFromModes(autoOnly),
        )

        val offOnly = intArrayOf(
            android.hardware.camera2.CaptureRequest.CONTROL_AF_MODE_OFF,
        )
        assertEquals(
            android.hardware.camera2.CaptureRequest.CONTROL_AF_MODE_OFF,
            CameraController.selectAfModeFromModes(offOnly),
        )

        val empty = intArrayOf()
        assertEquals(
            android.hardware.camera2.CaptureRequest.CONTROL_AF_MODE_OFF,
            CameraController.selectAfModeFromModes(empty),
        )
    }

    @Test
    fun `noise reduction, edge mode, and hot pixel resolve HIGH_QUALITY with FAST fallback`() {
        // Noise reduction
        val nrBoth = intArrayOf(
            android.hardware.camera2.CaptureRequest.NOISE_REDUCTION_MODE_FAST,
            android.hardware.camera2.CaptureRequest.NOISE_REDUCTION_MODE_HIGH_QUALITY,
        )
        assertEquals(
            android.hardware.camera2.CaptureRequest.NOISE_REDUCTION_MODE_HIGH_QUALITY,
            CameraController.resolveNoiseReductionModeFromModes(nrBoth),
        )

        val nrFastOnly = intArrayOf(
            android.hardware.camera2.CaptureRequest.NOISE_REDUCTION_MODE_FAST,
            android.hardware.camera2.CaptureRequest.NOISE_REDUCTION_MODE_OFF,
        )
        assertEquals(
            android.hardware.camera2.CaptureRequest.NOISE_REDUCTION_MODE_FAST,
            CameraController.resolveNoiseReductionModeFromModes(nrFastOnly),
        )

        val nrNone = intArrayOf(android.hardware.camera2.CaptureRequest.NOISE_REDUCTION_MODE_OFF)
        assertNull(CameraController.resolveNoiseReductionModeFromModes(nrNone))

        // Edge mode
        val edgeBoth = intArrayOf(
            android.hardware.camera2.CaptureRequest.EDGE_MODE_FAST,
            android.hardware.camera2.CaptureRequest.EDGE_MODE_HIGH_QUALITY,
        )
        assertEquals(
            android.hardware.camera2.CaptureRequest.EDGE_MODE_HIGH_QUALITY,
            CameraController.resolveEdgeModeFromModes(edgeBoth),
        )

        val edgeFastOnly = intArrayOf(android.hardware.camera2.CaptureRequest.EDGE_MODE_FAST)
        assertEquals(
            android.hardware.camera2.CaptureRequest.EDGE_MODE_FAST,
            CameraController.resolveEdgeModeFromModes(edgeFastOnly),
        )

        val edgeNone = intArrayOf(android.hardware.camera2.CaptureRequest.EDGE_MODE_OFF)
        assertNull(CameraController.resolveEdgeModeFromModes(edgeNone))

        // Hot pixel mode
        val hpBoth = intArrayOf(
            android.hardware.camera2.CaptureRequest.HOT_PIXEL_MODE_FAST,
            android.hardware.camera2.CaptureRequest.HOT_PIXEL_MODE_HIGH_QUALITY,
        )
        assertEquals(
            android.hardware.camera2.CaptureRequest.HOT_PIXEL_MODE_HIGH_QUALITY,
            CameraController.resolveHotPixelModeFromModes(hpBoth),
        )

        val hpFastOnly = intArrayOf(android.hardware.camera2.CaptureRequest.HOT_PIXEL_MODE_FAST)
        assertEquals(
            android.hardware.camera2.CaptureRequest.HOT_PIXEL_MODE_FAST,
            CameraController.resolveHotPixelModeFromModes(hpFastOnly),
        )

        val hpNone = intArrayOf(android.hardware.camera2.CaptureRequest.HOT_PIXEL_MODE_OFF)
        assertNull(CameraController.resolveHotPixelModeFromModes(hpNone))
    }

    @Test
    fun `face detect mode resolves FULL then SIMPLE then null`() {
        val faceBoth = intArrayOf(
            android.hardware.camera2.CaptureRequest.STATISTICS_FACE_DETECT_MODE_OFF,
            android.hardware.camera2.CaptureRequest.STATISTICS_FACE_DETECT_MODE_SIMPLE,
            android.hardware.camera2.CaptureRequest.STATISTICS_FACE_DETECT_MODE_FULL,
        )
        assertEquals(
            android.hardware.camera2.CaptureRequest.STATISTICS_FACE_DETECT_MODE_FULL,
            CameraController.resolveFaceDetectModeFromModes(faceBoth),
        )

        val faceSimpleOnly = intArrayOf(
            android.hardware.camera2.CaptureRequest.STATISTICS_FACE_DETECT_MODE_OFF,
            android.hardware.camera2.CaptureRequest.STATISTICS_FACE_DETECT_MODE_SIMPLE,
        )
        assertEquals(
            android.hardware.camera2.CaptureRequest.STATISTICS_FACE_DETECT_MODE_SIMPLE,
            CameraController.resolveFaceDetectModeFromModes(faceSimpleOnly),
        )

        val faceOffOnly = intArrayOf(
            android.hardware.camera2.CaptureRequest.STATISTICS_FACE_DETECT_MODE_OFF,
        )
        assertNull(CameraController.resolveFaceDetectModeFromModes(faceOffOnly))

        val faceEmpty = intArrayOf()
        assertNull(CameraController.resolveFaceDetectModeFromModes(faceEmpty))
    }
}
