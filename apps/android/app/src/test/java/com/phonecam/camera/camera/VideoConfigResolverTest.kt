package com.phonecam.camera.camera

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Capability-based resolution selection (PRD §13, Rule 5, Rule 6).
 *
 * This is the logic that decides what the product does when a phone cannot do
 * what was asked. It is a pure function precisely so it can be tested without a
 * device: the fallback behaviour is the part most likely to regress, and the part
 * PRD Rule 6 is most explicit about.
 */
class VideoConfigResolverTest {

    /** A typical mid-range phone: 1080p and 720p, but no 60 fps. */
    private val midRangeSizes = listOf(1920 to 1080, 1280 to 720, 640 to 480)
    private val midRangeFps = listOf(24, 30)

    /** A low-end phone that tops out at 720p. */
    private val lowEndSizes = listOf(1280 to 720, 640 to 480)
    private val lowEndFps = listOf(30)

    @Test
    fun `honours a supported request without substitution`() {
        val resolved = VideoConfigResolver.resolve(
            requestedResolution = "1280x720",
            requestedFps = 30,
            requestedBitrate = null,
            supportedSizes = midRangeSizes,
            supportedFps = midRangeFps,
        )

        assertEquals(1280, resolved.width)
        assertEquals(720, resolved.height)
        assertEquals(30, resolved.fps)
        assertFalse(resolved.substituted)
        assertNull(resolved.notice)
    }

    @Test
    fun `substitutes an unsupported resolution and explains why`() {
        val resolved = VideoConfigResolver.resolve(
            requestedResolution = "1920x1080",
            requestedFps = 30,
            requestedBitrate = null,
            supportedSizes = lowEndSizes,
            supportedFps = lowEndFps,
        )

        // PRD §38 wants the user told what happened, in the product's voice.
        assertEquals(1280, resolved.width)
        assertEquals(720, resolved.height)
        assertTrue(resolved.substituted)
        assertEquals("1080p isn't supported by this camera. Using 720p instead.", resolved.notice)
    }

    @Test
    fun `substitutes an unsupported frame rate without downgrading resolution`() {
        val resolved = VideoConfigResolver.resolve(
            requestedResolution = "1920x1080",
            requestedFps = 60,
            requestedBitrate = null,
            supportedSizes = midRangeSizes,
            supportedFps = midRangeFps,
        )

        // Resolution is honoured; only the frame rate gives way.
        assertEquals(1920, resolved.width)
        assertEquals(30, resolved.fps)
        assertTrue(resolved.substituted)
        assertEquals("60fps isn't supported by this camera. Using 30 fps instead.", resolved.notice)
    }

    @Test
    fun `never silently raises the frame rate above the request`() {
        // Asking for 24 fps on a 60 fps capable phone must stay at 24, not jump
        // to 60, which would triple the bitrate without being asked.
        val resolved = VideoConfigResolver.resolve(
            requestedResolution = "1280x720",
            requestedFps = 24,
            requestedBitrate = null,
            supportedSizes = listOf(1920 to 1080, 1280 to 720),
            supportedFps = listOf(24, 30, 60),
        )
        assertEquals(24, resolved.fps)
    }

    @Test
    fun `picks the largest supported size for an auto request`() {
        val resolved = VideoConfigResolver.resolve(
            requestedResolution = "auto",
            requestedFps = null,
            requestedBitrate = null,
            supportedSizes = midRangeSizes,
            supportedFps = midRangeFps,
        )
        assertEquals(1920, resolved.width)
        assertEquals(1080, resolved.height)
    }

    @Test
    fun `caps an auto frame rate at 30 for stability`() {
        // PRD §47 prefers a stable 30 over a less stable 60 when the user has not
        // asked for anything specific.
        val resolved = VideoConfigResolver.resolve(
            requestedResolution = "auto",
            requestedFps = null,
            requestedBitrate = null,
            supportedSizes = midRangeSizes,
            supportedFps = listOf(24, 30, 60),
        )
        assertEquals(30, resolved.fps)
    }

    @Test
    fun `degrades gracefully when the camera reports no capabilities`() {
        // A camera that failed to report must not crash the stream; the product
        // starts at the most conservative preset and can be corrected later.
        val resolved = VideoConfigResolver.resolve(
            requestedResolution = "1920x1080",
            requestedFps = 30,
            requestedBitrate = null,
            supportedSizes = emptyList(),
            supportedFps = emptyList(),
        )
        assertEquals(VideoPreset.LOW.width, resolved.width)
        assertEquals(VideoPreset.LOW.height, resolved.height)
        assertTrue(resolved.substituted)
        assertNotNull(resolved.notice)
    }

    @Test
    fun `honours an explicit bitrate over the preset default`() {
        val resolved = VideoConfigResolver.resolve(
            requestedResolution = "1280x720",
            requestedFps = 30,
            requestedBitrate = 2_500_000,
            supportedSizes = midRangeSizes,
            supportedFps = midRangeFps,
        )
        assertEquals(2_500_000, resolved.bitrate)
    }

    @Test
    fun `derives a bitrate from the chosen preset when none is requested`() {
        val resolved = VideoConfigResolver.resolve(
            requestedResolution = "1920x1080",
            requestedFps = 30,
            requestedBitrate = null,
            supportedSizes = midRangeSizes,
            supportedFps = midRangeFps,
        )
        assertEquals(VideoPreset.HIGH.targetBitrate, resolved.bitrate)
    }

    @Test
    fun `normalises a portrait request to landscape`() {
        // The encoder always emits sensor-native landscape, so a portrait ask
        // resolves to the matching landscape size.
        val resolved = VideoConfigResolver.resolve(
            requestedResolution = "1080x1920",
            requestedFps = 30,
            requestedBitrate = null,
            supportedSizes = midRangeSizes,
            supportedFps = midRangeFps,
        )
        assertEquals(1920, resolved.width)
        assertEquals(1080, resolved.height)
    }

    @Test
    fun `parses resolution strings and rejects malformed ones`() {
        assertEquals(1280 to 720, VideoPreset.parseResolution("1280x720"))
        assertNull(VideoPreset.parseResolution("720p"))
        assertNull(VideoPreset.parseResolution(""))
        assertNull(VideoPreset.parseResolution("1280x"))
        assertNull(VideoPreset.parseResolution("axb"))
    }

    @Test
    fun `nearestFps prefers a supported rate at or below the request`() {
        assertEquals(30, VideoConfigResolver.nearestFps(30, listOf(24, 30, 60)))
        assertEquals(24, VideoConfigResolver.nearestFps(25, listOf(24, 30, 60)))
        assertEquals(30, VideoConfigResolver.nearestFps(45, listOf(24, 30, 60)))
        // Below every supported rate, the lowest supported rate is the only
        // honest answer rather than inventing an unsupported one.
        assertEquals(24, VideoConfigResolver.nearestFps(10, listOf(24, 30)))
    }
}
