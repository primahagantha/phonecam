package com.phonecam.camera.session

import com.phonecam.camera.nativecore.NativeCore
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import java.io.Closeable

@Serializable
data class AdaptiveQualityDecision(
    val action: String,
    val resolution: String,
    val fps: Int,
    val bitrate: Int,
    val label: String,
    val reason: String,
) {
    val isChange: Boolean
        get() = action == "step_down" || action == "step_up"
}

/**
 * Lifecycle-safe wrapper around the Rust `AdaptiveQuality` controller via [NativeCore].
 *
 * Thread-safe on the native handle: [close] frees the boxed Rust instance at most
 * once, and subsequent calls become safe no-ops / `"hold"` decisions.
 */
class NativeAdaptiveQuality(
    enabled: Boolean = true,
    initResolution: String = "1920x1080",
    initFps: Int = 30,
) : Closeable {

    private val lock = Any()

    @Volatile
    private var handle: Long = NativeCore.adaptiveQualityCreate(enabled, initResolution, initFps)

    val isOpen: Boolean
        get() = handle != 0L

    fun setEnabled(enabled: Boolean) {
        synchronized(lock) {
            val h = handle
            if (h != 0L) {
                NativeCore.adaptiveQualitySetEnabled(h, enabled)
            }
        }
    }

    fun confirm(resolution: String, fps: Int) {
        synchronized(lock) {
            val h = handle
            if (h != 0L) {
                NativeCore.adaptiveQualityConfirm(h, resolution, fps)
            }
        }
    }

    fun observe(
        nowMs: Long,
        currentResolution: String,
        currentFps: Int,
        maxResolution: String,
        maxFps: Int,
        droppedPercent: Double,
        rttUs: Long = 0L,
        thermal: String = "none",
        decodeStruggling: Boolean = false,
    ): AdaptiveQualityDecision {
        val raw = synchronized(lock) {
            val h = handle
            if (h == 0L) {
                return AdaptiveQualityDecision(
                    action = "hold",
                    resolution = currentResolution,
                    fps = currentFps,
                    bitrate = 0,
                    label = "",
                    reason = "",
                )
            }
            NativeCore.adaptiveQualityObserve(
                handle = h,
                nowMs = nowMs,
                curRes = currentResolution,
                curFps = currentFps,
                maxRes = maxResolution,
                maxFps = maxFps,
                droppedPct = droppedPercent,
                rttUs = rttUs,
                thermal = thermal,
                decodeStruggling = decodeStruggling,
            )
        }
        return cachedHoldDecision(raw)
            ?: json.decodeFromString(AdaptiveQualityDecision.serializer(), raw)
    }

    override fun close() {
        synchronized(lock) {
            val h = handle
            if (h != 0L) {
                handle = 0L
                NativeCore.adaptiveQualityDestroy(h)
            }
        }
    }

    private companion object {
        val json = Json { ignoreUnknownKeys = true }

        val HOLD_480P = AdaptiveQualityDecision("hold", "640x480", 30, 1_500_000, "480p", "")
        val HOLD_720P = AdaptiveQualityDecision("hold", "1280x720", 30, 4_000_000, "720p", "")
        val HOLD_1080P30 = AdaptiveQualityDecision("hold", "1920x1080", 30, 8_000_000, "1080p30", "")
        val HOLD_1080P60 = AdaptiveQualityDecision("hold", "1920x1080", 60, 12_000_000, "1080p60", "")

        fun cachedHoldDecision(raw: String): AdaptiveQualityDecision? = when (raw) {
            """{"action":"hold","resolution":"1920x1080","fps":30,"bitrate":8000000,"label":"1080p30","reason":""}""" -> HOLD_1080P30
            """{"action":"hold","resolution":"1280x720","fps":30,"bitrate":4000000,"label":"720p","reason":""}""" -> HOLD_720P
            """{"action":"hold","resolution":"1920x1080","fps":60,"bitrate":12000000,"label":"1080p60","reason":""}""" -> HOLD_1080P60
            """{"action":"hold","resolution":"640x480","fps":30,"bitrate":1500000,"label":"480p","reason":""}""" -> HOLD_480P
            else -> null
        }
    }
}
