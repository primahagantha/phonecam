package com.phonecam.camera.service

import android.content.Intent
import com.phonecam.camera.session.SessionIntents
import com.phonecam.camera.session.snapOrientationDegrees
import com.phonecam.camera.ui.ScreenMode
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AndroidAuditFixesTest {

    @Test
    fun `isLoopbackHost only accepts loopback addresses`() {
        assertTrue(CameraStreamService.isLoopbackHost("127.0.0.1"))
        assertTrue(CameraStreamService.isLoopbackHost("localhost"))
        assertTrue(CameraStreamService.isLoopbackHost("LOCALHOST"))
        assertTrue(CameraStreamService.isLoopbackHost("::1"))
        assertTrue(CameraStreamService.isLoopbackHost(" 127.0.0.1 "))

        assertFalse(CameraStreamService.isLoopbackHost("192.168.1.50"))
        assertFalse(CameraStreamService.isLoopbackHost("10.0.0.1"))
        assertFalse(CameraStreamService.isLoopbackHost(""))
        assertFalse(CameraStreamService.isLoopbackHost(null))
    }

    @Test
    fun `external intents without internal token are restricted to loopback hosts`() {
        val externalLanIntent = Intent(SessionIntents.ACTION_CONNECT_MANUAL).apply {
            putExtra(SessionIntents.EXTRA_HOST, "192.168.1.50")
        }
        assertFalse(CameraStreamService.isAuthorizedConnectTarget(externalLanIntent, "192.168.1.50"))

        val externalLoopbackIntent = Intent(SessionIntents.ACTION_CONNECT_MANUAL).apply {
            putExtra(SessionIntents.EXTRA_HOST, "127.0.0.1")
        }
        assertTrue(CameraStreamService.isAuthorizedConnectTarget(externalLoopbackIntent, "127.0.0.1"))

        val forgedTokenIntent = Intent(SessionIntents.ACTION_CONNECT_TOKEN).apply {
            putExtra(SessionIntents.EXTRA_INTERNAL_TOKEN, "forged-token")
            putExtra(SessionIntents.EXTRA_HOST, "192.168.1.50")
        }
        assertFalse(CameraStreamService.isAuthorizedConnectTarget(forgedTokenIntent, "192.168.1.50"))
    }

    @Test
    fun `screen modes black and dim keep FLAG_KEEP_SCREEN_ON active`() {
        assertTrue(ScreenMode.KEEP_ON.keepScreenOn)
        assertTrue(ScreenMode.DIM.keepScreenOn)
        assertTrue(ScreenMode.BLACK.keepScreenOn)
    }

    @Test
    fun `snapOrientationDegrees snaps physical angles to right angles`() {
        assertNull(snapOrientationDegrees(-1))
        assertEquals(0, snapOrientationDegrees(0))
        assertEquals(0, snapOrientationDegrees(350))
        assertEquals(90, snapOrientationDegrees(45))
        assertEquals(90, snapOrientationDegrees(90))
        assertEquals(180, snapOrientationDegrees(180))
        assertEquals(270, snapOrientationDegrees(270))
    }
}
