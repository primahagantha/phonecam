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
}
