package com.phonecam.camera.session

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class NativeAdaptiveQualityTest {

    @Test
    fun `steps down after dwell on sustained high drop rate and holds while pending`() {
        NativeAdaptiveQuality(enabled = true, initResolution = "1920x1080", initFps = 30).use { aq ->
            val t0 = aq.observe(
                nowMs = 0L,
                currentResolution = "1920x1080",
                currentFps = 30,
                maxResolution = "1920x1080",
                maxFps = 30,
                droppedPercent = 5.0,
            )
            assertEquals("hold", t0.action)

            val t5 = aq.observe(
                nowMs = 5_000L,
                currentResolution = "1920x1080",
                currentFps = 30,
                maxResolution = "1920x1080",
                maxFps = 30,
                droppedPercent = 5.0,
            )
            assertEquals("hold", t5.action)

            val t10 = aq.observe(
                nowMs = 10_000L,
                currentResolution = "1920x1080",
                currentFps = 30,
                maxResolution = "1920x1080",
                maxFps = 30,
                droppedPercent = 5.0,
            )
            assertEquals("step_down", t10.action)
            assertEquals("1280x720", t10.resolution)
            assertEquals(30, t10.fps)
            assertEquals(4_000_000, t10.bitrate)
            assertTrue(t10.isChange)

            // Pending target holds until confirmed.
            val unconfirmed = aq.observe(
                nowMs = 25_000L,
                currentResolution = "1280x720",
                currentFps = 30,
                maxResolution = "1920x1080",
                maxFps = 30,
                droppedPercent = 5.0,
            )
            assertEquals("hold", unconfirmed.action)

            aq.confirm("1280x720", 30)
            val afterConfirm = aq.observe(
                nowMs = 26_000L,
                currentResolution = "1280x720",
                currentFps = 30,
                maxResolution = "1920x1080",
                maxFps = 30,
                droppedPercent = 5.0,
            )
            assertEquals("hold", afterConfirm.action)
            val secondStep = aq.observe(
                nowMs = 36_000L,
                currentResolution = "1280x720",
                currentFps = 30,
                maxResolution = "1920x1080",
                maxFps = 30,
                droppedPercent = 5.0,
            )
            assertEquals("step_down", secondStep.action)
            assertEquals("640x480", secondStep.resolution)
        }
    }

    @Test
    fun `steps down after dwell on slow RTT`() {
        NativeAdaptiveQuality(enabled = true, initResolution = "1920x1080", initFps = 30).use { aq ->
            assertEquals(
                "hold",
                aq.observe(
                    nowMs = 0L,
                    currentResolution = "1920x1080",
                    currentFps = 30,
                    maxResolution = "1920x1080",
                    maxFps = 30,
                    droppedPercent = 0.0,
                    rttUs = 150_000L,
                ).action,
            )

            val decision = aq.observe(
                nowMs = 10_000L,
                currentResolution = "1920x1080",
                currentFps = 30,
                maxResolution = "1920x1080",
                maxFps = 30,
                droppedPercent = 0.0,
                rttUs = 150_000L,
            )
            assertEquals("step_down", decision.action)
            assertEquals("1280x720", decision.resolution)
        }
    }

    @Test
    fun `steps down immediately on critical thermal stress and after dwell on severe thermal`() {
        NativeAdaptiveQuality(enabled = true, initResolution = "1920x1080", initFps = 30).use { aq ->
            val critical = aq.observe(
                nowMs = 0L,
                currentResolution = "1920x1080",
                currentFps = 30,
                maxResolution = "1920x1080",
                maxFps = 30,
                droppedPercent = 0.0,
                thermal = "critical",
            )
            assertEquals("step_down", critical.action)
            assertEquals("1280x720", critical.resolution)
        }

        NativeAdaptiveQuality(enabled = true, initResolution = "1920x1080", initFps = 30).use { aq ->
            assertEquals(
                "hold",
                aq.observe(
                    nowMs = 0L,
                    currentResolution = "1920x1080",
                    currentFps = 30,
                    maxResolution = "1920x1080",
                    maxFps = 30,
                    droppedPercent = 0.0,
                    thermal = "severe",
                ).action,
            )
            val afterDwell = aq.observe(
                nowMs = 10_000L,
                currentResolution = "1920x1080",
                currentFps = 30,
                maxResolution = "1920x1080",
                maxFps = 30,
                droppedPercent = 0.0,
                thermal = "severe",
            )
            assertEquals("step_down", afterDwell.action)
            assertEquals("1280x720", afterDwell.resolution)
        }
    }

    @Test
    fun `steps up after sustained clean stream up to max ceiling`() {
        NativeAdaptiveQuality(enabled = true, initResolution = "1280x720", initFps = 30).use { aq ->
            assertEquals(
                "hold",
                aq.observe(
                    nowMs = 0L,
                    currentResolution = "1280x720",
                    currentFps = 30,
                    maxResolution = "1920x1080",
                    maxFps = 30,
                    droppedPercent = 0.1,
                    rttUs = 15_000L,
                    thermal = "none",
                ).action,
            )
            assertEquals(
                "hold",
                aq.observe(
                    nowMs = 29_999L,
                    currentResolution = "1280x720",
                    currentFps = 30,
                    maxResolution = "1920x1080",
                    maxFps = 30,
                    droppedPercent = 0.1,
                    rttUs = 15_000L,
                    thermal = "none",
                ).action,
            )
            val stepUp = aq.observe(
                nowMs = 30_000L,
                currentResolution = "1280x720",
                currentFps = 30,
                maxResolution = "1920x1080",
                maxFps = 30,
                droppedPercent = 0.1,
                rttUs = 15_000L,
                thermal = "none",
            )
            assertEquals("step_up", stepUp.action)
            assertEquals("1920x1080", stepUp.resolution)
            assertEquals(30, stepUp.fps)
            assertEquals(8_000_000, stepUp.bitrate)
        }
    }

    @Test
    fun `manual mode holds even under severe drops and critical thermal`() {
        val qualityMode = "manual"
        NativeAdaptiveQuality(
            enabled = qualityMode != "manual",
            initResolution = "1920x1080",
            initFps = 30,
        ).use { aq ->
            val d1 = aq.observe(
                nowMs = 0L,
                currentResolution = "1920x1080",
                currentFps = 30,
                maxResolution = "1920x1080",
                maxFps = 30,
                droppedPercent = 25.0,
                rttUs = 400_000L,
                thermal = "critical",
                decodeStruggling = true,
            )
            assertEquals("hold", d1.action)
            assertFalse(d1.isChange)

            val d2 = aq.observe(
                nowMs = 15_000L,
                currentResolution = "1920x1080",
                currentFps = 30,
                maxResolution = "1920x1080",
                maxFps = 30,
                droppedPercent = 25.0,
                rttUs = 400_000L,
                thermal = "critical",
                decodeStruggling = true,
            )
            assertEquals("hold", d2.action)
            assertFalse(d2.isChange)
        }
    }

    @Test
    fun `close is idempotent and subsequent observe holds safely`() {
        val aq = NativeAdaptiveQuality(enabled = true, initResolution = "1280x720", initFps = 30)
        assertTrue(aq.isOpen)
        aq.close()
        assertFalse(aq.isOpen)
        aq.close()
        val afterClose = aq.observe(
            nowMs = 0L,
            currentResolution = "1280x720",
            currentFps = 30,
            maxResolution = "1920x1080",
            maxFps = 30,
            droppedPercent = 50.0,
            thermal = "critical",
        )
        assertEquals("hold", afterClose.action)
    }
}
