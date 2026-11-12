package com.phonecam.camera.camera

/**
 * The quality ladder from PRD §14.
 *
 * These are product targets, not guarantees: [nearestSupported] resolves a
 * requested preset to what the device can actually do, and the UI surfaces the
 * substitution rather than failing (PRD §13, §38, Rule 6).
 */
enum class VideoPreset(
    val width: Int,
    val height: Int,
    val fps: Int,
    val targetBitrate: Int,
) {
    LOW(640, 480, 30, 1_500_000),
    BALANCED(1280, 720, 30, 4_000_000),
    HIGH(1920, 1080, 30, 8_000_000),

    /**
     * 60 fps is only reachable through a Camera2 *high speed* session, and every
     * device that has one offers it at 720p (the sensor reads out fewer pixels to
     * hit the rate). Asking for 1080p60 is asking for a session the camera does
     * not have, so the preset names the size the rate actually comes with.
     */
    HIGH_FPS(1280, 720, 60, 12_000_000),
    ;

    val label: String
        get() = when (this) {
            LOW -> "480p30"
            BALANCED -> "720p30"
            HIGH -> "1080p30"
            HIGH_FPS -> "720p60"
        }

    val resolution: String
        get() = "${width}x$height"

    /** Ladder order used by adaptive quality (PRD §48): worst first. */
    val ladderRank: Int
        get() = ordinal

    companion object {
        /** Ordered from best to worst, which is the order adaptive quality steps down. */
        val descending: List<VideoPreset> = listOf(HIGH_FPS, HIGH, BALANCED, LOW)

        fun fromResolution(width: Int, height: Int, fps: Int): VideoPreset? =
            entries.firstOrNull { it.width == width && it.height == height && it.fps == fps }

        /** Parses `"1280x720"`; returns `null` when malformed. */
        fun parseResolution(value: String): Pair<Int, Int>? {
            val match = Regex("^(\\d{2,5})x(\\d{2,5})$").find(value.trim()) ?: return null
            val width = match.groupValues[1].toIntOrNull() ?: return null
            val height = match.groupValues[2].toIntOrNull() ?: return null
            return width to height
        }
    }
}

/** A concrete capture configuration the device has been proven to support. */
data class ResolvedVideoConfig(
    val width: Int,
    val height: Int,
    val fps: Int,
    val bitrate: Int,
    /** True when the requested configuration had to be substituted. */
    val substituted: Boolean,
    /** The user-facing explanation, or `null` when the request was honoured. */
    val notice: String?,
)

/**
 * Capability-aware selection.
 *
 * Inputs are the sizes and frame rates reported by the camera and the H.264
 * encoder for a given camera id. Nothing is assumed about the device: PRD Rule 5
 * forbids presuming that every Android phone does 1080p60.
 */
object VideoConfigResolver {

    /**
     * Resolves [requestedResolution] / [requestedFps] / [requestedBitrate] to a
     * supported configuration, preferring the largest size that fits the request
     * and the closest frame rate at or below it.
     */
    fun resolve(
        requestedResolution: String?,
        requestedFps: Int?,
        requestedBitrate: Int?,
        supportedSizes: List<Pair<Int, Int>>,
        supportedFps: List<Int>,
        /**
         * Sizes the camera can do in a high speed session. A frame rate above the
         * normal 30 is only reachable at one of these, so a 60 fps request is
         * snapped to the nearest of them rather than passed through at a size the
         * high speed session would reject.
         */
        highSpeedSizes: List<Pair<Int, Int>> = emptyList(),
    ): ResolvedVideoConfig {
        if (supportedSizes.isEmpty()) {
            // No capability data yet: fall back to the most conservative preset
            // so the stream can start and be corrected once caps arrive.
            return ResolvedVideoConfig(
                width = VideoPreset.LOW.width,
                height = VideoPreset.LOW.height,
                fps = supportedFps.minOrNull() ?: 30,
                bitrate = VideoPreset.LOW.targetBitrate,
                substituted = true,
                notice = "This camera didn't report its supported sizes. Using 480p.",
            )
        }

        val target = requestedResolution?.let { VideoPreset.parseResolution(it) }
        val chosenSize = when {
            target == null -> largest(supportedSizes)
            else -> nearestSize(target.first, target.second, supportedSizes)
        }

        val requested = requestedFps?.takeIf { it > 0 }
        val chosenFps = when {
            requested == null -> supportedFps.maxOrNull()?.coerceAtMost(30) ?: 30
            supportedFps.isEmpty() -> requested
            else -> nearestFps(requested, supportedFps)
        }

        // The high speed session fixes the size, so the size follows the rate
        // rather than the other way round.
        val finalSize = if (chosenFps >= 60 && highSpeedSizes.isNotEmpty()) {
            nearestSize(chosenSize.first, chosenSize.second, highSpeedSizes)
        } else {
            chosenSize
        }

        val preset = VideoPreset.entries.firstOrNull {
            it.width == finalSize.first && it.height == finalSize.second && it.fps == chosenFps
        }
        val defaultBitrate = preset?.targetBitrate
            ?: bitrateFor(finalSize.first, finalSize.second, chosenFps)
        val bitrate = requestedBitrate ?: defaultBitrate

        val resolutionSubstituted = target != null && target != finalSize
        val fpsSubstituted = requested != null && requested != chosenFps

        val notice = when {
            resolutionSubstituted && fpsSubstituted ->
                "${label(target.first, target.second)}${requested}fps isn't supported by this camera. " +
                    "Using ${label(finalSize.first, finalSize.second)}$chosenFps instead."
            resolutionSubstituted ->
                "${label(target.first, target.second)} isn't supported by this camera. " +
                    "Using ${label(finalSize.first, finalSize.second)} instead."
            fpsSubstituted ->
                "${requested}fps isn't supported by this camera. Using $chosenFps fps instead."
            else -> null
        }

        return ResolvedVideoConfig(
            width = finalSize.first,
            height = finalSize.second,
            fps = chosenFps,
            bitrate = bitrate,
            substituted = resolutionSubstituted || fpsSubstituted,
            notice = notice,
        )
    }

    /** The largest supported size, preferring landscape orientation. */
    private fun largest(sizes: List<Pair<Int, Int>>): Pair<Int, Int> =
        sizes.maxByOrNull { it.first.toLong() * it.second } ?: sizes.first()

    /**
     * Picks the supported size closest to [width] x [height], comparing by area
     * with a preference for exact matches and for landscape orientation so the
     * virtual camera output stays consistent (PRD §49).
     */
    fun nearestSize(width: Int, height: Int, sizes: List<Pair<Int, Int>>): Pair<Int, Int> {
        val landscape = sizes.map { (w, h) -> if (w >= h) w to h else h to w }
        landscape.firstOrNull { it.first == width && it.second == height }?.let { return it }

        val targetArea = width.toLong() * height
        val targetAspect = width.toDouble() / height
        return landscape.minByOrNull { (w, h) ->
            val area = w.toLong() * h
            val areaPenalty = if (area > targetArea) (area - targetArea) * 3 else targetArea - area
            val aspectPenalty = (kotlin.math.abs(w.toDouble() / h - targetAspect) * targetArea).toLong()
            areaPenalty + aspectPenalty
        } ?: landscape.first()
    }

    /**
     * Chooses the supported frame rate nearest to [requested], preferring one at
     * or below the request so a 30 fps ask never silently becomes 60 fps.
     */
    fun nearestFps(requested: Int, supported: List<Int>): Int {
        val sorted = supported.distinct().sorted()
        sorted.firstOrNull { it == requested }?.let { return it }
        return sorted.lastOrNull { it <= requested }
            ?: sorted.firstOrNull()
            ?: requested
    }

    private fun bitrateFor(width: Int, height: Int, fps: Int): Int {
        val perPixel = 0.09
        val raw = (width.toLong() * height * fps * perPixel).toLong()
        return raw.coerceIn(800_000L, 20_000_000L).toInt()
    }

    private fun label(width: Int, height: Int): String = "${minOf(width, height)}p"
}
